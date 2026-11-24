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

/// Scope classification for Muninn memory tiers (`SDD-1D`, `F-04`).
///
/// Prevents blindly imposing single-user `OwnerScope` onto shared knowledge
/// while enforcing strict isolation for private and administrative records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryKind {
    Conversation,
    Message,
    Preference,
    ReasoningTrace,
    ReasoningStep,
    ToolCall,
    SharedKnowledge,
    ReferenceEntity,
    AuditRecord,
}

/// Access boundary class derived from [`MemoryKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryScopeClass {
    Private,
    Workspace,
    Global,
    RestrictedAdmin,
}

/// Operation kind evaluated by [`MemoryAccessPolicy`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccessOperation {
    Read,
    Write,
}

/// Verified caller identity and administrative privilege at a trust boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessContext {
    pub caller: OwnerScope,
    pub workspace: Option<String>,
    pub is_admin: bool,
}

impl AccessContext {
    pub const fn user(caller: OwnerScope) -> Self {
        Self {
            caller,
            workspace: None,
            is_admin: false,
        }
    }

    pub const fn admin(caller: OwnerScope) -> Self {
        Self {
            caller,
            workspace: None,
            is_admin: true,
        }
    }

    pub fn with_workspace(mut self, workspace: &str) -> MemoryResult<Self> {
        if workspace.trim().is_empty() {
            return Err(MemoryError::invalid(
                "workspace identifier must not be blank",
            ));
        }
        self.workspace = Some(workspace.to_owned());
        Ok(self)
    }
}

/// Target resource descriptor evaluated against [`AccessContext`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetResource {
    pub kind: MemoryKind,
    pub owner: OwnerScope,
    pub workspace: Option<String>,
}

impl TargetResource {
    pub const fn private(kind: MemoryKind, owner: OwnerScope) -> Self {
        Self {
            kind,
            owner,
            workspace: None,
        }
    }

    pub fn workspace(kind: MemoryKind, workspace: &str) -> MemoryResult<Self> {
        if workspace.trim().is_empty() {
            return Err(MemoryError::invalid(
                "workspace identifier must not be blank",
            ));
        }
        Ok(Self {
            kind,
            owner: OwnerScope::unowned(),
            workspace: Some(workspace.to_owned()),
        })
    }

    pub const fn global(kind: MemoryKind) -> Self {
        Self {
            kind,
            owner: OwnerScope::unowned(),
            workspace: None,
        }
    }

    pub const fn restricted_admin(kind: MemoryKind) -> Self {
        Self {
            kind,
            owner: OwnerScope::unowned(),
            workspace: None,
        }
    }
}

/// Central authorization policy mapping [`MemoryKind`] to [`MemoryScopeClass`].
pub struct MemoryAccessPolicy;

impl MemoryAccessPolicy {
    pub const fn classify(kind: MemoryKind) -> MemoryScopeClass {
        match kind {
            MemoryKind::Conversation
            | MemoryKind::Message
            | MemoryKind::Preference
            | MemoryKind::ReasoningTrace
            | MemoryKind::ReasoningStep
            | MemoryKind::ToolCall => MemoryScopeClass::Private,
            MemoryKind::SharedKnowledge => MemoryScopeClass::Workspace,
            MemoryKind::ReferenceEntity => MemoryScopeClass::Global,
            MemoryKind::AuditRecord => MemoryScopeClass::RestrictedAdmin,
        }
    }

    pub fn authorize(
        operation: AccessOperation,
        context: &AccessContext,
        target: &TargetResource,
    ) -> MemoryResult<()> {
        match Self::classify(target.kind) {
            MemoryScopeClass::Private => {
                if context.caller == target.owner {
                    Ok(())
                } else {
                    Err(MemoryError::invalid(
                        "cross-owner access to private memory is forbidden",
                    ))
                }
            }
            MemoryScopeClass::Workspace => match (&context.workspace, &target.workspace) {
                (Some(caller_ws), Some(target_ws))
                    if !caller_ws.trim().is_empty() && caller_ws == target_ws =>
                {
                    Ok(())
                }
                _ => Err(MemoryError::invalid(
                    "workspace scope mismatch for shared knowledge",
                )),
            },
            MemoryScopeClass::Global => match operation {
                AccessOperation::Read => Ok(()),
                AccessOperation::Write if context.is_admin => Ok(()),
                AccessOperation::Write => Err(MemoryError::invalid(
                    "administrative privilege required to mutate global reference entities",
                )),
            },
            MemoryScopeClass::RestrictedAdmin => {
                if context.is_admin {
                    Ok(())
                } else {
                    Err(MemoryError::invalid(
                        "administrative privilege required for restricted audit records",
                    ))
                }
            }
        }
    }
}

/// Validated WGS-84 geographic coordinate pair (`BOLT-08-03`, `BOLT-08-14`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeoPoint {
    pub latitude: f64,
    pub longitude: f64,
}

impl GeoPoint {
    pub fn new(latitude: f64, longitude: f64) -> MemoryResult<Self> {
        if !latitude.is_finite() || !(-90.0..=90.0).contains(&latitude) {
            return Err(MemoryError::invalid(
                "latitude must be finite and within [-90.0, 90.0]",
            ));
        }
        if !longitude.is_finite() || !(-180.0..=180.0).contains(&longitude) {
            return Err(MemoryError::invalid(
                "longitude must be finite and within [-180.0, 180.0]",
            ));
        }
        Ok(Self {
            latitude,
            longitude,
        })
    }
}

/// Validated geographic bounding box (`BOLT-08-03`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoundingBox {
    pub min_lat: f64,
    pub min_lon: f64,
    pub max_lat: f64,
    pub max_lon: f64,
}

impl BoundingBox {
    pub fn new(min_lat: f64, min_lon: f64, max_lat: f64, max_lon: f64) -> MemoryResult<Self> {
        let _ = GeoPoint::new(min_lat, min_lon)?;
        let _ = GeoPoint::new(max_lat, max_lon)?;
        if min_lat > max_lat || min_lon > max_lon {
            return Err(MemoryError::invalid(
                "bounding box min coordinates must not exceed max coordinates",
            ));
        }
        Ok(Self {
            min_lat,
            min_lon,
            max_lat,
            max_lon,
        })
    }
}

/// Spatial search hit ordered by distance (`BOLT-08-03`).
#[derive(Debug, Clone)]
pub struct SpatialLocationHit {
    pub entity: Entity,
    pub coordinates: GeoPoint,
    pub distance_km: f64,
}

/// Session-scoped location projection via `EXTRACTED_FROM` (`BOLT-08-13`).
#[derive(Debug, Clone)]
pub struct SessionLocation {
    pub entity: Entity,
    pub coordinates: Option<GeoPoint>,
    pub conversation_ids: Vec<Uuid>,
}

/// Persistence port for geographic retrieval and location projection (`BOLT-08-03/13/14`).
#[async_trait]
pub trait SpatialStore: Send + Sync {
    async fn search_locations_near(
        &self,
        center: GeoPoint,
        radius_km: f64,
        limit: i64,
    ) -> MemoryResult<Vec<SpatialLocationHit>>;

    async fn search_locations_in_bounding_box(
        &self,
        bbox: BoundingBox,
        limit: i64,
    ) -> MemoryResult<Vec<SpatialLocationHit>>;

    async fn get_location_coordinates(&self, entity_id: Uuid) -> MemoryResult<Option<GeoPoint>>;

    async fn get_locations(
        &self,
        session_id: Option<&str>,
        limit: i64,
    ) -> MemoryResult<Vec<SessionLocation>>;
}

/// Validate a Cypher label or property identifier (`[A-Za-z_][A-Za-z0-9_]*`).
///
/// Rejects any character that could break out of backtick interpolation or
/// inject Cypher clauses before any query is built (`BOLT-08-06`, `SDD-3`).
pub fn is_safe_cypher_identifier(ident: &str) -> bool {
    let mut chars = ident.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Options for non-destructive existing graph adoption (`BOLT-08-06`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdoptGraphOptions {
    /// Map from existing Neo4j node label -> Muninn entity type (e.g. `Person -> PERSON`).
    pub label_to_type: std::collections::BTreeMap<String, String>,
    /// Optional override for the source name property per label (defaults to `"name"`).
    pub name_property_per_label: std::collections::BTreeMap<String, String>,
    /// Defaults to `true` (plan-only, zero mutations).
    pub dry_run: bool,
}

impl Default for AdoptGraphOptions {
    fn default() -> Self {
        Self {
            label_to_type: std::collections::BTreeMap::new(),
            name_property_per_label: std::collections::BTreeMap::new(),
            dry_run: true,
        }
    }
}

impl AdoptGraphOptions {
    /// Stage 1 (`Validate`): validate every label, property, and entity type upfront.
    pub fn validate(&self) -> MemoryResult<()> {
        for (label, entity_type) in &self.label_to_type {
            if !is_safe_cypher_identifier(label) {
                return Err(MemoryError::invalid(format!(
                    "refusing to adopt label {label:?}: must match [A-Za-z_][A-Za-z0-9_]*"
                )));
            }
            if entity_type.trim().is_empty() {
                return Err(MemoryError::invalid(format!(
                    "entity_type for label {label:?} must not be blank"
                )));
            }
            let name_prop = self
                .name_property_per_label
                .get(label)
                .map(String::as_str)
                .unwrap_or("name");
            if !is_safe_cypher_identifier(name_prop) {
                return Err(MemoryError::invalid(format!(
                    "refusing to use name property {name_prop:?} for label {label:?}: must match [A-Za-z_][A-Za-z0-9_]*"
                )));
            }
        }
        for (label, name_prop) in &self.name_property_per_label {
            if !is_safe_cypher_identifier(label) || !is_safe_cypher_identifier(name_prop) {
                return Err(MemoryError::invalid(
                    "invalid identifier in name_property_per_label",
                ));
            }
        }
        Ok(())
    }

    pub fn name_property_for<'a>(&'a self, label: &str) -> &'a str {
        self.name_property_per_label
            .get(label)
            .map(String::as_str)
            .unwrap_or("name")
    }
}

/// Per-label outcome reported by [`SchemaAdoptionPort::adopt_existing_graph`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdoptionLabelReport {
    pub label: String,
    pub entity_type: String,
    pub name_property: String,
    pub migrated_count: i64,
    pub already_adopted_count: i64,
    pub skipped_count: i64,
    pub conflict_count: i64,
}

/// Structured report returned by [`SchemaAdoptionPort::adopt_existing_graph`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AdoptionReport {
    pub by_label: Vec<AdoptionLabelReport>,
    pub dry_run: bool,
}

impl AdoptionReport {
    pub fn total_migrated(&self) -> i64 {
        self.by_label.iter().map(|r| r.migrated_count).sum()
    }

    pub fn total_conflicts(&self) -> i64 {
        self.by_label.iter().map(|r| r.conflict_count).sum()
    }

    pub fn has_conflicts(&self) -> bool {
        self.total_conflicts() > 0
    }
}

/// Port for non-destructive adoption of pre-existing Neo4j graphs (`BOLT-08-06`).
#[async_trait]
pub trait SchemaAdoptionPort: Send + Sync {
    async fn adopt_existing_graph(
        &self,
        options: &AdoptGraphOptions,
    ) -> MemoryResult<AdoptionReport>;
}

/// A write that failed in the background buffered drainer (`BOLT-08-04`).
///
/// Parameter values are intentionally omitted from the stored error record to
/// avoid retaining sensitive payloads or credentials in memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BufferedWriteError {
    pub query: String,
    pub parameter_keys: Vec<String>,
    pub error: String,
    pub when: DateTime<Utc>,
}

/// A queued buffered write operation (`BOLT-08-04`).
#[derive(Debug, Clone)]
pub struct BufferedWriteJob {
    pub query: String,
    pub params: serde_json::Map<String, serde_json::Value>,
    /// Retries are permitted only when the caller explicitly marks the write idempotent.
    pub idempotent: bool,
}

impl BufferedWriteJob {
    pub fn new(
        query: impl Into<String>,
        params: serde_json::Map<String, serde_json::Value>,
    ) -> Self {
        Self {
            query: query.into(),
            params,
            idempotent: false,
        }
    }

    pub fn idempotent(
        query: impl Into<String>,
        params: serde_json::Map<String, serde_json::Value>,
    ) -> Self {
        Self {
            query: query.into(),
            params,
            idempotent: true,
        }
    }
}

/// Low-level write execution port consumed by [`BufferedWriter`] (`BOLT-08-04`).
#[async_trait]
pub trait WriteExecutor: Send + Sync {
    async fn execute_write(
        &self,
        query: &str,
        params: &serde_json::Map<String, serde_json::Value>,
    ) -> MemoryResult<()>;
}

/// Prepared extracted entity and mention provenance for single-transaction ingestion.
#[derive(Debug, Clone)]
pub struct AtomicExtractionEntity {
    pub entity: Entity,
    pub provenance: crate::extraction::MentionProvenance,
}

/// Prepared extracted relationship between two entities in the same ingestion batch.
#[derive(Debug, Clone)]
pub struct AtomicExtractionRelation {
    pub source_name: String,
    pub target_name: String,
    pub relation_type: String,
    pub confidence: f64,
}

/// Prepared extraction mutations committed atomically alongside a message (`SDD-1C`, `F-01`).
#[derive(Debug, Clone, Default)]
pub struct AtomicExtractionBatch {
    pub existing_mentions: Vec<(Uuid, crate::extraction::MentionProvenance)>,
    pub entities: Vec<AtomicExtractionEntity>,
    pub relations: Vec<AtomicExtractionRelation>,
}

impl AtomicExtractionBatch {
    pub fn is_empty(&self) -> bool {
        self.existing_mentions.is_empty() && self.entities.is_empty() && self.relations.is_empty()
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

    /// Atomically append a message and persist its prepared extraction batch in
    /// a single storage transaction (`SDD-1C`, `F-01`).
    ///
    /// Production stores (e.g., Neo4j) return `Ok(Some(message))` after
    /// committing both message and extraction writes together (or roll back all
    /// writes on error). In-memory test doubles that do not span both stores
    /// may return `Ok(None)` to use the test-only split path.
    async fn append_message_atomic(
        &self,
        _conversation_id: Uuid,
        _scope: &OwnerScope,
        _message: &MessageRecord,
        _extraction: &AtomicExtractionBatch,
    ) -> MemoryResult<Option<Message>> {
        Ok(None)
    }

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

impl ConsolidationReport {
    /// Enforce single transactional audit ownership invariants (`SDD-4`, `F-05`):
    /// - Dry-run must have zero mutations and no `run_id`.
    /// - Execute with zero mutations must have no `run_id`.
    /// - Execute with mutations must have a persisted `run_id` written atomically by the store.
    pub fn validate_invariants(&self) -> MemoryResult<()> {
        if self.dry_run {
            if self.mutations_applied != 0 {
                return Err(MemoryError::storage(
                    "consolidation dry-run reported non-zero mutations_applied",
                ));
            }
            if self.run_id.is_some() {
                return Err(MemoryError::storage(
                    "consolidation dry-run must not persist a ConsolidationRun audit node",
                ));
            }
            return Ok(());
        }
        if self.mutations_applied == 0 && self.run_id.is_some() {
            return Err(MemoryError::storage(
                "consolidation job with zero mutations_applied must not persist a run_id",
            ));
        }
        if self.mutations_applied > 0 && self.run_id.is_none() {
            return Err(MemoryError::storage(
                "consolidation job applied mutations without an atomic ConsolidationRun run_id",
            ));
        }
        Ok(())
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

    /// Owner-scoped variant of [`Self::detect_superseded_preferences`] (`SDD-4`, `F-09`).
    async fn detect_superseded_preferences_for_owner(
        &self,
        user_identifier: Option<&str>,
        threshold: f64,
        limit: i64,
        dry_run: bool,
    ) -> MemoryResult<ConsolidationReport> {
        if let Some(owner) = user_identifier {
            if owner.trim().is_empty() {
                return Err(MemoryError::invalid("user_identifier must not be blank"));
            }
            return Err(MemoryError::NotSupported(
                "owner-filtered preference supersession".into(),
            ));
        }
        self.detect_superseded_preferences(threshold, limit, dry_run)
            .await
    }

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
        is_safe_cypher_identifier, AccessContext, AccessOperation, AdoptGraphOptions,
        AdoptionLabelReport, AdoptionReport, AtomicExtractionBatch, BoundingBox,
        ConsolidationReport, EmbeddingIdentity, GeoPoint, MemoryAccessPolicy, MemoryKind,
        MemoryScopeClass, MessageRecord, OwnerScope, SessionOrder, SessionQuery, TargetResource,
        TraceOrder, TraceQuery,
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

    #[test]
    fn memory_access_policy_classifies_all_memory_kinds() {
        for kind in [
            MemoryKind::Conversation,
            MemoryKind::Message,
            MemoryKind::Preference,
            MemoryKind::ReasoningTrace,
            MemoryKind::ReasoningStep,
            MemoryKind::ToolCall,
        ] {
            assert_eq!(
                MemoryAccessPolicy::classify(kind),
                MemoryScopeClass::Private
            );
        }
        assert_eq!(
            MemoryAccessPolicy::classify(MemoryKind::SharedKnowledge),
            MemoryScopeClass::Workspace
        );
        assert_eq!(
            MemoryAccessPolicy::classify(MemoryKind::ReferenceEntity),
            MemoryScopeClass::Global
        );
        assert_eq!(
            MemoryAccessPolicy::classify(MemoryKind::AuditRecord),
            MemoryScopeClass::RestrictedAdmin
        );
    }

    #[test]
    fn memory_access_policy_enforces_private_workspace_global_and_admin_boundaries() {
        let alice = OwnerScope::owned("alice").unwrap();
        let bob = OwnerScope::owned("bob").unwrap();

        let alice_ctx = AccessContext::user(alice.clone())
            .with_workspace("ws-1")
            .unwrap();
        let bob_ctx = AccessContext::user(bob.clone())
            .with_workspace("ws-2")
            .unwrap();
        let admin_ctx = AccessContext::admin(bob.clone())
            .with_workspace("ws-1")
            .unwrap();

        // Private: only matching owner may read/write (even admin cannot cross-read private tenant data)
        let alice_msg = TargetResource::private(MemoryKind::Message, alice.clone());
        assert!(
            MemoryAccessPolicy::authorize(AccessOperation::Read, &alice_ctx, &alice_msg).is_ok()
        );
        assert!(
            MemoryAccessPolicy::authorize(AccessOperation::Write, &alice_ctx, &alice_msg).is_ok()
        );
        assert!(
            MemoryAccessPolicy::authorize(AccessOperation::Read, &bob_ctx, &alice_msg).is_err()
        );
        assert!(
            MemoryAccessPolicy::authorize(AccessOperation::Read, &admin_ctx, &alice_msg).is_err()
        );

        // Workspace: matching workspace allowed, cross-workspace denied, missing workspace denied
        let ws1_knowledge = TargetResource::workspace(MemoryKind::SharedKnowledge, "ws-1").unwrap();
        assert!(
            MemoryAccessPolicy::authorize(AccessOperation::Read, &alice_ctx, &ws1_knowledge)
                .is_ok()
        );
        assert!(
            MemoryAccessPolicy::authorize(AccessOperation::Read, &bob_ctx, &ws1_knowledge).is_err()
        );
        let no_ws_ctx = AccessContext::user(alice.clone());
        assert!(
            MemoryAccessPolicy::authorize(AccessOperation::Read, &no_ws_ctx, &ws1_knowledge)
                .is_err()
        );

        // Global: read-shared for any caller, write requires admin
        let global_ref = TargetResource::global(MemoryKind::ReferenceEntity);
        assert!(
            MemoryAccessPolicy::authorize(AccessOperation::Read, &alice_ctx, &global_ref).is_ok()
        );
        assert!(
            MemoryAccessPolicy::authorize(AccessOperation::Read, &bob_ctx, &global_ref).is_ok()
        );
        assert!(
            MemoryAccessPolicy::authorize(AccessOperation::Write, &alice_ctx, &global_ref).is_err()
        );
        assert!(
            MemoryAccessPolicy::authorize(AccessOperation::Write, &admin_ctx, &global_ref).is_ok()
        );

        // RestrictedAdmin: read & write require admin
        let audit = TargetResource::restricted_admin(MemoryKind::AuditRecord);
        assert!(MemoryAccessPolicy::authorize(AccessOperation::Read, &alice_ctx, &audit).is_err());
        assert!(MemoryAccessPolicy::authorize(AccessOperation::Write, &alice_ctx, &audit).is_err());
        assert!(MemoryAccessPolicy::authorize(AccessOperation::Read, &admin_ctx, &audit).is_ok());
        assert!(MemoryAccessPolicy::authorize(AccessOperation::Write, &admin_ctx, &audit).is_ok());

        // Blank workspace IDs fail closed
        assert!(AccessContext::user(alice).with_workspace("   ").is_err());
        assert!(TargetResource::workspace(MemoryKind::SharedKnowledge, " ").is_err());
    }

    #[test]
    fn atomic_extraction_batch_reports_emptiness() {
        let batch = AtomicExtractionBatch::default();
        assert!(batch.is_empty());
    }

    #[test]
    fn adopt_graph_options_validate_rejects_cypher_injection_and_defaults_dry_run() {
        let defaults = AdoptGraphOptions::default();
        assert!(defaults.dry_run);
        assert!(defaults.validate().is_ok());

        for bad in [
            "",
            "123Label",
            "Label; MATCH (n) DETACH DELETE n",
            "Bad`Label",
            "With Space",
            "foo-bar",
        ] {
            assert!(!is_safe_cypher_identifier(bad), "should reject {bad:?}");
            let mut opts = AdoptGraphOptions::default();
            opts.label_to_type
                .insert(bad.to_owned(), "PERSON".to_owned());
            assert!(opts.validate().is_err(), "label {bad:?} must be rejected");
        }

        for good in ["Person", "User_Account_2", "_Internal"] {
            assert!(is_safe_cypher_identifier(good), "should accept {good:?}");
        }

        let mut bad_prop = AdoptGraphOptions::default();
        bad_prop
            .label_to_type
            .insert("Person".into(), "PERSON".into());
        bad_prop
            .name_property_per_label
            .insert("Person".into(), "name` RETURN 1 //".into());
        assert!(bad_prop.validate().is_err());

        let report = AdoptionReport {
            by_label: vec![AdoptionLabelReport {
                label: "Person".into(),
                entity_type: "PERSON".into(),
                name_property: "name".into(),
                migrated_count: 3,
                already_adopted_count: 1,
                skipped_count: 2,
                conflict_count: 1,
            }],
            dry_run: true,
        };
        assert_eq!(report.total_migrated(), 3);
        assert_eq!(report.total_conflicts(), 1);
        assert!(report.has_conflicts());
    }

    #[test]
    fn geo_point_and_bounding_box_validate_bounds_fail_closed() {
        assert!(GeoPoint::new(40.4168, -3.7038).is_ok());
        assert!(GeoPoint::new(90.0, 180.0).is_ok());
        assert!(GeoPoint::new(-90.0, -180.0).is_ok());
        assert!(GeoPoint::new(90.1, 0.0).is_err());
        assert!(GeoPoint::new(0.0, -180.1).is_err());
        assert!(GeoPoint::new(f64::NAN, 0.0).is_err());
        assert!(GeoPoint::new(0.0, f64::INFINITY).is_err());

        assert!(BoundingBox::new(35.0, -10.0, 44.0, 5.0).is_ok());
        assert!(BoundingBox::new(45.0, -10.0, 44.0, 5.0).is_err());
        assert!(BoundingBox::new(35.0, 6.0, 44.0, 5.0).is_err());
    }
}
