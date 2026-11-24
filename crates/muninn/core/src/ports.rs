//! Ports implemented by infrastructure adapters.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::error::{MemoryError, MemoryResult};
use crate::model::{
    Conversation, Entity, EntityGraph, EntityMention, EntityRef, Fact, Message, Metadata,
    NewMessage, Observation, Preference, ReasoningStep, ReasoningStepWithContext, ReasoningTrace,
    Relationship, SessionInfo, ToolCall, ToolStats,
};

/// Stable identity of a vector space. The key must pin the model revision and
/// preprocessing/pooling/normalization contract, not merely its dimensions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingIdentity {
    key: String,
    dimensions: usize,
}

impl EmbeddingIdentity {
    pub fn new(key: impl Into<String>, dimensions: usize) -> MemoryResult<Self> {
        let key = key.into();
        if key.trim().is_empty() || dimensions == 0 || dimensions > i64::MAX as usize {
            return Err(crate::MemoryError::invalid("invalid embedding identity"));
        }
        Ok(Self { key, dimensions })
    }

    pub fn key(&self) -> &str {
        &self.key
    }
    pub const fn dimensions(&self) -> usize {
        self.dimensions
    }
}

/// Text embedding provider (`embeddings.base.Embedder`).
#[async_trait]
pub trait Embedder: Send + Sync {
    /// Vector dimensions produced by this embedder.
    fn dimensions(&self) -> usize;

    /// Unidentified providers remain usable directly, but cannot persist or search
    /// short-term vectors. Implementations must explicitly identify their space.
    fn identity(&self) -> MemoryResult<EmbeddingIdentity> {
        Err(crate::MemoryError::invalid("embedder identity is required"))
    }

    async fn embed(&self, text: &str) -> MemoryResult<Vec<f64>>;

    async fn embed_batch(&self, texts: &[String]) -> MemoryResult<Vec<Vec<f64>>> {
        let mut vectors = Vec::with_capacity(texts.len());
        for text in texts {
            vectors.push(self.embed(text).await?);
        }
        Ok(vectors)
    }
}

/// A message ready to persist: identity, content and optional vector.
#[derive(Debug, Clone)]
pub struct MessageRecord {
    pub operation_key: Option<String>,
    pub operation_payload: Option<String>,
    pub id: Uuid,
    pub role: String,
    pub content: String,
    pub embedding: Option<Vec<f64>>,
    pub embedding_identity: Option<EmbeddingIdentity>,
    pub metadata: Metadata,
    pub timestamp: Option<DateTime<Utc>>,
}

impl MessageRecord {
    pub fn from_new(message: &NewMessage, embedding: Option<Vec<f64>>) -> Self {
        Self {
            operation_key: None,
            operation_payload: None,
            id: Uuid::new_v4(),
            role: message.role.as_str().to_owned(),
            content: message.content.clone(),
            embedding,
            embedding_identity: None,
            metadata: message.metadata.clone(),
            timestamp: message.timestamp,
        }
    }
}

/// A persisted message found through its operation key.
#[derive(Debug, Clone)]
pub struct KeyedMessage {
    pub message: Message,
    pub operation_payload: Option<String>,
}

/// Ordering for [`ShortTermStore::list_sessions`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SessionOrder {
    CreatedAt,
    #[default]
    UpdatedAt,
    MessageCount,
}

impl SessionOrder {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CreatedAt => "created_at",
            Self::UpdatedAt => "updated_at",
            Self::MessageCount => "message_count",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SessionQuery {
    pub prefix: Option<String>,
    pub limit: i64,
    pub offset: i64,
    pub order_by: SessionOrder,
    pub ascending: bool,
}

/// Caller-provided conversation owner scope.
///
/// It selects a namespace; it is
/// not authentication or authorization. `None` denotes the separate unowned
/// namespace, never a global namespace that spans every owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerScope(Option<String>);

impl OwnerScope {
    pub const fn unowned() -> Self {
        Self(None)
    }

    pub fn owned(owner: &str) -> MemoryResult<Self> {
        if owner.trim().is_empty() {
            return Err(MemoryError::invalid("user_identifier must not be blank"));
        }
        Ok(Self(Some(owner.to_owned())))
    }

    pub fn from_option(owner: Option<&str>) -> MemoryResult<Self> {
        owner.map_or_else(|| Ok(Self::unowned()), Self::owned)
    }

    pub const fn is_unowned(&self) -> bool {
        self.0.is_none()
    }

    pub fn as_str(&self) -> Option<&str> {
        self.0.as_deref()
    }

    /// Collision-free storage encoding: the unowned namespace can never equal
    /// an owned namespace, even if an owner's identifier collides with a
    /// sentinel value such as "unowned".
    pub fn storage_key(&self) -> String {
        self.0
            .as_ref()
            .map_or_else(|| "unowned".to_owned(), |owner| format!("owned:{owner}"))
    }

    /// Decode a stored owner_scope back into a scope. Legacy rows whose stored
    /// value can never be produced by [`Self::storage_key`] are rejected instead of
    /// being silently adopted into a namespace.
    pub fn from_storage_key(raw: &str) -> MemoryResult<Self> {
        if raw == "unowned" {
            return Ok(Self::unowned());
        }
        raw.strip_prefix("owned:")
            .ok_or_else(|| MemoryError::invalid("invalid stored owner namespace"))
            .and_then(Self::owned)
    }
}

/// A message hit returned by vector search.
#[derive(Debug, Clone)]
pub struct ScoredMessage {
    pub message: Message,
    pub score: f64,
}

/// Persistence port for short-term (conversation) memory.
#[async_trait]
pub trait ShortTermStore: Send + Sync {
    /// Return the conversation id for the `(scope, session_id)` namespace,
    /// creating it when absent. Never adopts or transfers a conversation
    /// owned by another namespace.
    async fn ensure_conversation(
        &self,
        session_id: &str,
        conversation_id: Option<Uuid>,
        scope: &OwnerScope,
    ) -> MemoryResult<Uuid>;

    /// Find a message written under `operation_key` in this conversation
    /// namespace. The returned payload lets the application distinguish a
    /// replay from a conflicting reuse before it calls an embedding provider.
    async fn find_by_operation_key(
        &self,
        conversation_id: Uuid,
        scope: &OwnerScope,
        operation_key: &str,
    ) -> MemoryResult<Option<KeyedMessage>>;

    /// Append one message to the tail of a conversation's chain. The scope
    /// must match the conversation's namespace.
    async fn append_message(
        &self,
        conversation_id: Uuid,
        scope: &OwnerScope,
        message: &MessageRecord,
    ) -> MemoryResult<Message>;

    /// Append messages in input order in one transaction. The scope must
    /// match the conversation's namespace.
    async fn append_messages(
        &self,
        conversation_id: Uuid,
        scope: &OwnerScope,
        messages: &[MessageRecord],
    ) -> MemoryResult<Vec<Message>>;

    async fn get_conversation(
        &self,
        session_id: &str,
        conversation_id: Option<Uuid>,
        scope: &OwnerScope,
        limit: Option<i64>,
    ) -> MemoryResult<Option<Conversation>>;

    async fn get_conversation_by_id(
        &self,
        conversation_id: Uuid,
        scope: &OwnerScope,
    ) -> MemoryResult<Option<Conversation>>;

    async fn append_observation(
        &self,
        observation: &Observation,
        scope: &OwnerScope,
    ) -> MemoryResult<()>;

    async fn list_observations(
        &self,
        conversation_id: Uuid,
        scope: &OwnerScope,
        limit: i64,
        offset: i64,
    ) -> MemoryResult<Vec<Observation>>;

    async fn list_conversations(
        &self,
        scope: &OwnerScope,
        limit: i64,
    ) -> MemoryResult<Vec<Conversation>>;

    /// Identity-aware search must never fall back to an unidentified vector search.
    #[allow(clippy::too_many_arguments)]
    async fn search_messages_with_identity(
        &self,
        _embedding: &[f64],
        _identity: &EmbeddingIdentity,
        _session_id: Option<&str>,
        _scope: &OwnerScope,
        _limit: i64,
        _threshold: f64,
        _filters: &Metadata,
    ) -> MemoryResult<Vec<ScoredMessage>> {
        Err(MemoryError::invalid(
            "store does not support embedding identity",
        ))
    }
    async fn search_messages(
        &self,
        embedding: &[f64],
        session_id: Option<&str>,
        scope: &OwnerScope,
        limit: i64,
        threshold: f64,
        metadata_filters: &Metadata,
    ) -> MemoryResult<Vec<ScoredMessage>>;

    async fn list_sessions(
        &self,
        query: &SessionQuery,
        scope: &OwnerScope,
    ) -> MemoryResult<Vec<SessionInfo>>;

    /// Delete a message, repairing the chain. Returns whether it existed.
    /// A foreign or missing scope must not delete or reveal anything.
    async fn delete_message(
        &self,
        message_id: Uuid,
        scope: &OwnerScope,
        cascade: bool,
    ) -> MemoryResult<bool>;

    async fn clear_session(&self, session_id: &str, scope: &OwnerScope) -> MemoryResult<()>;

    async fn delete_conversation(
        &self,
        conversation_id: Uuid,
        scope: &OwnerScope,
    ) -> MemoryResult<()>;

    /// Return the top entity names mentioned in a session's conversation,
    /// ordered by mention count (for `get_conversation_summary`).
    async fn summary_entities(
        &self,
        session_id: &str,
        scope: &OwnerScope,
    ) -> MemoryResult<Vec<String>>;
}

/// Destructive maintenance operations, reserved for disposable test graphs.
#[async_trait]
pub trait GraphMaintenance: Send + Sync {
    /// Delete every node and relationship.
    async fn clear_all_data(&self) -> MemoryResult<()>;
}

/// A vector-search hit: the stored value and its similarity score.
#[derive(Debug, Clone)]
pub struct Scored<T> {
    pub item: T,
    pub score: f64,
}

/// Mutable fields of an entity update. `None` leaves the field untouched.
#[derive(Debug, Clone, Default)]
pub struct EntityUpdate {
    pub name: Option<String>,
    pub canonical_name: Option<String>,
    pub description: Option<String>,
    pub embedding: Option<Vec<f64>>,
}

/// An entity reached from another one, with the edge that links them.
#[derive(Debug, Clone)]
pub struct RelatedEntity {
    pub relationship: Relationship,
    pub entity: Entity,
}

/// A trace hit returned by vector search.
#[derive(Debug, Clone)]
pub struct ScoredTrace {
    pub trace: ReasoningTrace,
    pub score: f64,
}

/// A reasoning step with its parent conversation, when it has one.
#[derive(Debug, Clone)]
pub struct StepContext {
    pub conversation_id: Option<Uuid>,
    pub step: ReasoningStep,
    pub touched_entities: Vec<Entity>,
}

/// A reasoning step and the conversation it was recorded under.
#[derive(Debug, Clone)]
pub struct ConversationStep {
    pub conversation_id: Uuid,
    pub step: ReasoningStep,
}

/// Fields needed to create a reasoning trace.
#[derive(Debug, Clone)]
pub struct TraceRecord {
    pub id: Uuid,
    pub session_id: String,
    pub task: String,
    pub task_embedding: Option<Vec<f64>>,
    pub metadata: Metadata,
}

/// Fields needed to append a reasoning step.
#[derive(Debug, Clone)]
pub struct StepRecord {
    pub id: Uuid,
    pub thought: Option<String>,
    pub action: Option<String>,
    pub observation: Option<String>,
    pub embedding: Option<Vec<f64>>,
    pub metadata: Metadata,
}

/// Field `list_traces` orders by (`reasoning.list_traces`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TraceOrder {
    #[default]
    StartedAt,
    CompletedAt,
}

impl TraceOrder {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StartedAt => "started_at",
            Self::CompletedAt => "completed_at",
        }
    }
}

/// Filtering, pagination and ordering for [`ReasoningStore::list_traces`].
#[derive(Debug, Clone)]
pub struct TraceQuery {
    pub session_id: Option<String>,
    pub success: Option<bool>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub limit: i64,
    pub offset: i64,
    pub order_by: TraceOrder,
    /// Descending by default, as upstream `order_dir="desc"`.
    pub ascending: bool,
}

impl Default for TraceQuery {
    fn default() -> Self {
        Self {
            session_id: None,
            success: None,
            since: None,
            until: None,
            limit: 100,
            offset: 0,
            order_by: TraceOrder::StartedAt,
            ascending: false,
        }
    }
}

/// Persistence port for long-term (entity, relationship, preference and
/// fact) memory.
#[async_trait]
pub trait LongTermStore: Send + Sync {
    /// Return at most `limit` active candidates, exact surfaces first, then
    /// lexical/vector candidates. Apply the optional type restriction before
    /// limiting; an absent vector must still allow textual matching.
    /// Legacy adapters fail explicitly rather than silently resolving distinct.
    async fn resolution_candidates(
        &self,
        _query: &crate::resolution::ResolutionQuery<'_>,
    ) -> MemoryResult<Vec<Scored<Entity>>> {
        Err(MemoryError::NotSupported(
            "entity resolution candidates".into(),
        ))
    }

    /// Create an entity, or update the existing `(name, type)` node.
    async fn upsert_entity(&self, entity: &Entity) -> MemoryResult<Entity>;

    async fn get_entity(&self, id: Uuid) -> MemoryResult<Option<Entity>>;

    /// Look up by name, canonical name or alias.
    async fn get_entity_by_name(&self, name: &str) -> MemoryResult<Option<Entity>>;

    async fn search_entities(
        &self,
        embedding: &[f64],
        limit: i64,
        threshold: f64,
    ) -> MemoryResult<Vec<Scored<Entity>>>;

    async fn list_entities(
        &self,
        entity_type: Option<&str>,
        limit: i64,
    ) -> MemoryResult<Vec<Entity>>;

    async fn update_entity(&self, id: Uuid, update: &EntityUpdate) -> MemoryResult<Option<Entity>>;

    async fn delete_entity(&self, id: Uuid) -> MemoryResult<bool>;

    async fn related_entities(&self, id: Uuid, depth: u8) -> MemoryResult<Vec<RelatedEntity>>;

    /// Create (or reinforce) a typed edge. `None` when an endpoint is absent.
    async fn upsert_relationship(
        &self,
        relationship: &Relationship,
    ) -> MemoryResult<Option<Relationship>>;

    /// Keep `target`, copy `source`'s edges onto it and record `source`'s name
    /// as an alias. Returns both entities after the merge.
    async fn merge_entities(
        &self,
        source_id: Uuid,
        target_id: Uuid,
    ) -> MemoryResult<Option<(Entity, Entity)>>;

    async fn link_same_as(
        &self,
        source_id: Uuid,
        target_id: Uuid,
        confidence: f64,
        match_type: &str,
        status: &str,
    ) -> MemoryResult<()>;

    async fn set_entity_feedback(
        &self,
        id: Uuid,
        user_score: f64,
        confirmed: bool,
    ) -> MemoryResult<bool>;

    async fn link_message_to_entity(
        &self,
        message_id: Uuid,
        entity_id: Uuid,
        confidence: f64,
    ) -> MemoryResult<bool>;

    /// Additive evidence-aware entry point. Legacy implementations must not
    /// silently discard ingestion provenance. Implementations reuse their writer.
    async fn link_message_to_entity_with_provenance(
        &self,
        _message_id: Uuid,
        _entity_id: Uuid,
        _provenance: &crate::extraction::MentionProvenance,
    ) -> MemoryResult<bool> {
        Err(MemoryError::NotSupported("MENTIONS provenance".into()))
    }

    /// Every message that mentions the entity; `None` when it does not exist.
    async fn entity_mentions(&self, id: Uuid) -> MemoryResult<Option<Vec<EntityMention>>>;

    async fn entity_graph(&self, limit: i64) -> MemoryResult<EntityGraph>;

    /// Capacidad explícita: implementa write_preference y
    /// supersede_preference_atomic bajo el mismo protocolo compuesto.
    fn supports_atomic_preference_write(&self) -> bool {
        false
    }

    /// Deduplica y persiste la preferencia y sus relaciones de forma atómica.
    /// El adaptador debe serializar las búsquedas/escrituras concurrentes,
    /// rechazar destinos ausentes y revertir errores previos a la confirmación.
    /// Si no puede conocer el resultado de la confirmación, debe indicarlo sin
    /// prometer rollback. No se permite una alternativa con escrituras parciales.
    async fn write_preference(
        &self,
        _preference: &Preference,
        _user_identifier: Option<&str>,
        _applies_to: &[Uuid],
        _about_entities: &[Uuid],
        _duplicate_threshold: f64,
    ) -> MemoryResult<Preference> {
        Err(MemoryError::NotSupported("atomic preference write".into()))
    }

    /// Supersesión explícita bajo el mismo protocolo que write_preference.
    /// La capacidad atómica exige implementar ambas operaciones.
    async fn supersede_preference_atomic(&self, _old_id: Uuid, _new_id: Uuid) -> MemoryResult<()> {
        Err(MemoryError::NotSupported(
            "atomic preference supersession".into(),
        ))
    }

    async fn create_preference(&self, preference: &Preference) -> MemoryResult<Preference>;

    async fn find_duplicate_preference(
        &self,
        embedding: &[f64],
        category: &str,
        threshold: f64,
        user_identifier: Option<&str>,
    ) -> MemoryResult<Option<Preference>>;

    async fn link_user_to_preference(
        &self,
        user_identifier: &str,
        preference_id: Uuid,
    ) -> MemoryResult<()>;

    /// `(:Preference)-[:APPLIES_TO]->(:Entity)`, idempotent like upstream.
    async fn link_preference_to_entity(
        &self,
        preference_id: Uuid,
        entity_id: Uuid,
    ) -> MemoryResult<()>;

    /// `(:Preference)-[:ABOUT]->(:Entity)`, the pinned cross-memory edge
    /// (`graph/queries.py`: one property-free, idempotent `MERGE`). Additive:
    /// legacy implementations must fail explicitly instead of silently
    /// dropping a requested cross-memory link.
    async fn link_preference_about_entity(
        &self,
        _preference_id: Uuid,
        _entity_id: Uuid,
    ) -> MemoryResult<()> {
        Err(MemoryError::NotSupported("preference ABOUT".into()))
    }

    /// `(:Preference)-[:SUPERSEDED_BY]->(:Preference)` and `valid_until` on the
    /// old preference, exactly once; repeating the call is a no-op.
    async fn supersede_preference(&self, old_id: Uuid, new_id: Uuid) -> MemoryResult<()>;

    /// Preferences linked to a user, most recent first; `active_only` hides the
    /// ones already superseded.
    async fn preferences_for(
        &self,
        user_identifier: &str,
        active_only: bool,
    ) -> MemoryResult<Vec<Preference>>;

    /// AND target-edge and half-open validity filters with the user scope.
    /// Legacy implementations remain source-compatible, but must not ignore filters.
    async fn preferences_for_filtered(
        &self,
        user_identifier: &str,
        active_only: bool,
        applies_to: Option<Uuid>,
        as_of: Option<DateTime<Utc>>,
    ) -> MemoryResult<Vec<Preference>> {
        if applies_to.is_some() || as_of.is_some() {
            return Err(MemoryError::NotSupported("preference filters".into()));
        }
        self.preferences_for(user_identifier, active_only).await
    }

    async fn search_preferences(
        &self,
        embedding: &[f64],
        limit: i64,
        threshold: f64,
        user_identifier: Option<&str>,
    ) -> MemoryResult<Vec<Scored<Preference>>>;

    async fn preferences_by_category(
        &self,
        category: &str,
        limit: i64,
        user_identifier: Option<&str>,
    ) -> MemoryResult<Vec<Preference>>;

    async fn create_fact(&self, fact: &Fact) -> MemoryResult<Fact>;

    async fn find_duplicate_fact(
        &self,
        embedding: &[f64],
        subject: &str,
        predicate: &str,
        threshold: f64,
    ) -> MemoryResult<Option<Fact>>;

    async fn update_fact_confidence(
        &self,
        id: Uuid,
        confidence: f64,
        object: &str,
    ) -> MemoryResult<()>;

    async fn search_facts(
        &self,
        embedding: &[f64],
        limit: i64,
        threshold: f64,
    ) -> MemoryResult<Vec<Scored<Fact>>>;

    async fn facts_by_subject(&self, subject: &str, limit: i64) -> MemoryResult<Vec<Fact>>;

    /// Create the fact and all `(:Fact)-[:ABOUT]->(:Entity)` edges atomically.
    async fn create_fact_with_about(&self, fact: &Fact, about: &[Uuid]) -> MemoryResult<Fact> {
        if !about.is_empty() {
            return Err(MemoryError::NotSupported("atomic fact ABOUT".into()));
        }
        self.create_fact(fact).await
    }

    /// Update a duplicate and merge its requested edges in the same write.
    async fn update_fact_with_about(
        &self,
        id: Uuid,
        confidence: f64,
        object: &str,
        about: &[Uuid],
    ) -> MemoryResult<()> {
        if !about.is_empty() {
            return Err(MemoryError::NotSupported("atomic fact ABOUT".into()));
        }
        self.update_fact_confidence(id, confidence, object).await
    }
}

/// Persistence port for reasoning (trace, step, tool-call and hosted
/// conversation-step) memory.
#[async_trait]
pub trait ReasoningStore: Send + Sync {
    async fn create_trace(
        &self,
        trace: &TraceRecord,
        scope: &OwnerScope,
        message_id: Option<Uuid>,
    ) -> MemoryResult<ReasoningTrace>;

    async fn validate_message_scope(
        &self,
        message_id: Uuid,
        scope: &OwnerScope,
    ) -> MemoryResult<()>;
    async fn add_conversation_step(
        &self,
        conversation_id: Uuid,
        trace_id: Uuid,
        scope: &OwnerScope,
        step: &StepRecord,
    ) -> MemoryResult<ReasoningStep>;
    async fn link_user_to_trace(&self, user_identifier: &str, trace_id: Uuid) -> MemoryResult<()>;

    /// Append a step, assigning the next `step_number`.
    async fn add_step(&self, trace_id: Uuid, step: &StepRecord) -> MemoryResult<ReasoningStep>;

    async fn set_step_observation(&self, step_id: Uuid, observation: &str) -> MemoryResult<()>;

    async fn steps_without_embedding(&self, trace_id: Uuid) -> MemoryResult<Vec<ReasoningStep>>;

    async fn set_step_embedding(&self, step_id: Uuid, embedding: &[f64]) -> MemoryResult<()>;

    async fn record_tool_call(
        &self,
        call: &ToolCall,
        scope: &OwnerScope,
        message_id: Option<Uuid>,
    ) -> MemoryResult<()>;

    /// Set the outcome and `completed_at`. `None` when the trace is absent.
    async fn complete_trace(
        &self,
        trace_id: Uuid,
        outcome: Option<&str>,
        success: Option<bool>,
    ) -> MemoryResult<Option<ReasoningTrace>>;

    /// Persist only the structured `error_kind` and serialized metrics, after
    /// legacy completion. Older adapters fail explicitly rather than lose fields.
    async fn set_trace_outcome_fields(
        &self,
        _trace_id: Uuid,
        _outcome: &crate::model::TraceOutcome,
    ) -> MemoryResult<()> {
        Err(MemoryError::NotSupported(
            "structured trace outcome fields".into(),
        ))
    }

    /// Highest numbered step, not list order. Adapters may provide a narrow read.
    async fn last_trace_step(&self, trace_id: Uuid) -> MemoryResult<Option<Uuid>> {
        Ok(self
            .get_trace_with_steps(trace_id)
            .await?
            .and_then(|trace| {
                trace
                    .steps
                    .into_iter()
                    .max_by_key(|step| step.step_number)
                    .map(|step| step.id)
            }))
    }

    async fn get_trace_with_steps(&self, trace_id: Uuid) -> MemoryResult<Option<ReasoningTrace>>;

    async fn list_traces(&self, query: &TraceQuery) -> MemoryResult<Vec<ReasoningTrace>>;

    async fn tool_stats(&self, tool_name: Option<&str>) -> MemoryResult<Vec<ToolStats>>;

    async fn search_traces(
        &self,
        embedding: &[f64],
        limit: i64,
        threshold: f64,
        success_only: bool,
    ) -> MemoryResult<Vec<ScoredTrace>>;

    async fn search_steps(
        &self,
        embedding: &[f64],
        limit: i64,
        threshold: f64,
        success_only: bool,
    ) -> MemoryResult<Vec<ReasoningStepWithContext>>;

    async fn link_trace_to_message(
        &self,
        trace_id: Uuid,
        message_id: Uuid,
        scope: &OwnerScope,
    ) -> MemoryResult<bool>;

    async fn link_conversation_to_trace(
        &self,
        conversation_id: Uuid,
        trace_id: Uuid,
        scope: &OwnerScope,
    ) -> MemoryResult<bool>;

    async fn link_tool_call_to_message(
        &self,
        tool_call_id: Uuid,
        message_id: Uuid,
        scope: &OwnerScope,
    ) -> MemoryResult<bool>;

    /// `(:ReasoningStep)-[:TOUCHED]->(:Entity)`, resolving the reference.
    async fn record_touched(&self, step_id: Uuid, entity: &EntityRef) -> MemoryResult<bool>;

    async fn open_conversation_trace(
        &self,
        conversation_id: Uuid,
        scope: &OwnerScope,
    ) -> MemoryResult<Option<Uuid>>;

    /// Return the conversation's open trace, creating it when absent.
    async fn ensure_conversation_trace(
        &self,
        conversation_id: Uuid,
        trace: &TraceRecord,
        scope: &OwnerScope,
    ) -> MemoryResult<Uuid>;

    /// Steps of every trace linked to the conversation; `None` when the
    /// conversation does not exist.
    async fn conversation_steps(
        &self,
        conversation_id: Uuid,
        scope: &OwnerScope,
    ) -> MemoryResult<Option<Vec<ReasoningStep>>>;

    async fn step_context(
        &self,
        step_id: Uuid,
        scope: &OwnerScope,
    ) -> MemoryResult<Option<StepContext>>;

    async fn entity_provenance(
        &self,
        entity_id: &str,
        scope: &OwnerScope,
    ) -> MemoryResult<Vec<ConversationStep>>;
}

/// A consolidation candidate returned by hygiene jobs.
#[derive(Debug, Clone)]
pub struct ConsolidationCandidate {
    /// Unique identifier of the primary item (entity id, trace id, etc.).
    pub id: String,
    /// Type of consolidation: "merge", "summarize", "supersede", "archive".
    pub kind: String,
    /// Secondary identifier (target entity id, related trace id, etc.).
    pub target_id: Option<String>,
    /// Similarity score, confidence, or other metric.
    pub score: Option<f64>,
    /// Human-readable reason.
    pub reason: String,
}

/// Report returned by a consolidation job.
#[derive(Debug, Clone)]
pub struct ConsolidationReport {
    /// Job kind (e.g., "dedupe_entities", "summarize_long_traces").
    pub kind: String,
    /// Whether this was a dry run (no mutations).
    pub dry_run: bool,
    /// Number of candidates found.
    pub candidates_found: usize,
    /// Number of mutations applied (0 if dry_run).
    pub mutations_applied: usize,
    /// Detailed candidate list.
    pub candidates: Vec<ConsolidationCandidate>,
    /// Optional audit run ID written to the graph.
    pub run_id: Option<String>,
}

impl Default for ConsolidationReport {
    fn default() -> Self {
        Self {
            kind: String::new(),
            dry_run: true,
            candidates_found: 0,
            mutations_applied: 0,
            candidates: Vec::new(),
            run_id: None,
        }
    }
}

/// Request to record an explicit `:MemoryReadAudit` event (`BOLT-08-11`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadAuditRecord {
    /// Free-text label for the read (e.g. query string or API endpoint name).
    pub query: String,
    /// Optional `:User` identifier to link via `(:User)-[:PERFORMED_READ]->(:MemoryReadAudit)`.
    pub user_identifier: Option<String>,
    /// Audit category (e.g. `"memory.read"`, `"preference.read"`).
    pub kind: String,
    /// Optional record count returned by the read.
    pub result_count: Option<i64>,
    /// Optional JSON-serialized metadata attached to the audit node.
    pub metadata_json: Option<String>,
}

impl ReadAuditRecord {
    /// Construct a read audit record with default kind `"memory.read"`.
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            user_identifier: None,
            kind: "memory.read".to_owned(),
            result_count: None,
            metadata_json: None,
        }
    }
}

/// Persistence port for memory consolidation — idempotent, dry-runnable
/// hygiene jobs over the knowledge graph.
///
/// Equivalent to `neo4j_agent_memory.memory.consolidation.ConsolidationMemory`.
#[async_trait]
pub trait ConsolidationStore: Send + Sync {
    /// Find pairs of entities with high embedding similarity and either
    /// flag them with `:SAME_AS` or report them.
    async fn dedupe_entities(
        &self,
        threshold: f64,
        limit: i64,
        dry_run: bool,
    ) -> MemoryResult<ConsolidationReport>;

    /// Find traces with many steps that haven't been summarized yet.
    async fn summarize_long_traces(
        &self,
        min_steps: i64,
        limit: i64,
        dry_run: bool,
    ) -> MemoryResult<ConsolidationReport>;

    /// Find preferences with near-duplicate content suggesting one supersedes
    /// the other.
    async fn detect_superseded_preferences(
        &self,
        threshold: f64,
        limit: i64,
        dry_run: bool,
    ) -> MemoryResult<ConsolidationReport>;

    /// Archive conversations older than the given TTL.
    async fn archive_expired_conversations(
        &self,
        ttl_days: i64,
        limit: i64,
        dry_run: bool,
    ) -> MemoryResult<ConsolidationReport>;

    /// Persist a `:ConsolidationRun` audit node and return its run ID.
    async fn record_run(&self, kind: &str, stats_json: &str) -> MemoryResult<String>;

    /// Persist an explicit `:MemoryReadAudit` node and optionally link it to a
    /// `:User` via `[:PERFORMED_READ]`, returning the audit ID (`BOLT-08-11`).
    async fn record_read_audit(&self, record: &ReadAuditRecord) -> MemoryResult<String>;
}

#[cfg(test)]
mod owner_scope_tests {
    use super::{
        ConsolidationReport, EmbeddingIdentity, MessageRecord, OwnerScope, SessionOrder,
        SessionQuery, TraceOrder, TraceQuery,
    };
    use crate::model::{MessageRole, Metadata, NewMessage};
    use chrono::{DateTime, Utc};
    use serde_json::Value;

    #[test]
    fn owners_preserve_exact_identity_and_do_not_alias_unowned() {
        assert_ne!(
            OwnerScope::owned(" owner ").expect("owner is nonblank"),
            OwnerScope::owned("owner").expect("owner is nonblank")
        );
        assert!(OwnerScope::owned(" ").is_err());
        assert_ne!(
            OwnerScope::owned("unowned")
                .expect("owner is nonblank")
                .storage_key(),
            OwnerScope::unowned().storage_key()
        );
    }

    #[test]
    fn owner_scope_helpers_encode_and_decode_namespaces() {
        let unowned = OwnerScope::unowned();
        assert!(unowned.is_unowned());
        assert_eq!(unowned.as_str(), None);
        assert_eq!(unowned.storage_key(), "unowned");
        assert_eq!(
            OwnerScope::from_option(None).expect("unowned scope"),
            unowned
        );
        assert_eq!(
            OwnerScope::from_storage_key("unowned").expect("decode"),
            unowned
        );

        let owned = OwnerScope::owned("alice").expect("valid owner");
        assert!(!owned.is_unowned());
        assert_eq!(owned.as_str(), Some("alice"));
        assert_eq!(owned.storage_key(), "owned:alice");
        assert_eq!(
            OwnerScope::from_option(Some("alice")).expect("owned scope"),
            owned
        );
        assert_eq!(
            OwnerScope::from_storage_key("owned:alice").expect("decode"),
            owned
        );
        assert_eq!(
            OwnerScope::from_storage_key("alice")
                .expect_err("invalid namespace rejected")
                .to_string(),
            "invalid input: invalid stored owner namespace"
        );
        assert!(OwnerScope::from_storage_key("owned: ").is_err());
    }

    #[test]
    fn embedding_identity_validates_and_exposes_its_key_and_dimensions() {
        let identity = EmbeddingIdentity::new("model:v1", 3).expect("valid identity");
        assert_eq!(identity.key(), "model:v1");
        assert_eq!(identity.dimensions(), 3);
        assert_eq!(
            EmbeddingIdentity::new("  ", 3)
                .expect_err("blank key rejected")
                .to_string(),
            "invalid input: invalid embedding identity"
        );
        assert!(EmbeddingIdentity::new("model:v1", 0).is_err());
        assert!(EmbeddingIdentity::new("model:v1", usize::MAX).is_err());
    }

    #[test]
    fn session_orders_have_stable_names_and_query_defaults() {
        assert_eq!(SessionOrder::CreatedAt.as_str(), "created_at");
        assert_eq!(SessionOrder::UpdatedAt.as_str(), "updated_at");
        assert_eq!(SessionOrder::MessageCount.as_str(), "message_count");
        assert_eq!(SessionOrder::default(), SessionOrder::UpdatedAt);

        let query = SessionQuery::default();
        assert_eq!(query.prefix, None);
        assert_eq!(query.limit, 0);
        assert_eq!(query.offset, 0);
        assert_eq!(query.order_by, SessionOrder::UpdatedAt);
        assert!(!query.ascending);
    }

    #[test]
    fn trace_orders_and_query_defaults_are_stable() {
        assert_eq!(TraceOrder::StartedAt.as_str(), "started_at");
        assert_eq!(TraceOrder::CompletedAt.as_str(), "completed_at");
        assert_eq!(TraceOrder::default(), TraceOrder::StartedAt);

        let query = TraceQuery::default();
        assert_eq!(query.session_id, None);
        assert_eq!(query.success, None);
        assert_eq!(query.since, None);
        assert_eq!(query.until, None);
        assert_eq!(query.limit, 100);
        assert_eq!(query.offset, 0);
        assert_eq!(query.order_by, TraceOrder::StartedAt);
        assert!(!query.ascending);
    }

    #[test]
    fn consolidation_report_defaults_to_a_dry_run() {
        let report = ConsolidationReport::default();
        assert!(report.kind.is_empty());
        assert!(report.dry_run);
        assert_eq!(report.candidates_found, 0);
        assert_eq!(report.mutations_applied, 0);
        assert!(report.candidates.is_empty());
        assert_eq!(report.run_id, None);
    }

    #[test]
    fn message_record_from_new_copies_input_and_adds_empty_operation_fields() {
        let timestamp =
            DateTime::<Utc>::from_timestamp(1_700_000_000, 0).expect("timestamp is representable");
        let metadata = Metadata::from_iter([("source".to_owned(), Value::from("test"))]);
        let input = NewMessage {
            role: MessageRole::Assistant,
            content: "hello".to_owned(),
            metadata: metadata.clone(),
            timestamp: Some(timestamp),
        };
        let record = MessageRecord::from_new(&input, Some(vec![0.25, 0.75]));
        assert_eq!(record.role, "assistant");
        assert_eq!(record.content, "hello");
        assert_eq!(record.metadata, metadata);
        assert_eq!(record.timestamp, Some(timestamp));
        assert_eq!(record.embedding, Some(vec![0.25, 0.75]));
        assert_eq!(record.operation_key, None);
        assert_eq!(record.operation_payload, None);
        assert_eq!(record.embedding_identity, None);
    }
}
