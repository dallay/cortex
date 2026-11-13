//! `get_context`: the pinned fused prompt document over the bridge wire.
//!
//! The store double serves canned reads and applies the same owner filter the
//! Neo4j adapter applies, so a document that leaked another owner's message or
//! preference would fail here. Nothing reaches a database or a provider.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::Utc;
use muninn_application::{MemoryClient, MemoryOptions, MemoryPorts};
use muninn_bridge::{router, BridgeState};
use muninn_core::model::{
    Conversation, Entity, EntityGraph, EntityMention, EntityRef, Fact, Message, MessageRole,
    Metadata, Preference, ReasoningStep, ReasoningStepWithContext, ReasoningTrace, Relationship,
    SessionInfo, ToolCall, ToolStats,
};
use muninn_core::ports::{
    ConversationStep, EmbeddingIdentity, EntityUpdate, KeyedMessage, LongTermStore, MessageRecord,
    OwnerScope, ReasoningStore, RelatedEntity, Scored, ScoredMessage, ScoredTrace, SessionQuery,
    ShortTermStore, StepContext, StepRecord, TraceQuery, TraceRecord,
};
use muninn_core::MemoryResult;
use serde_json::json;
use tower::ServiceExt;
use uuid::Uuid;

/// Deterministic embedder: one dimension, one fixed vector, every call counted.
#[derive(Default)]
struct FixedEmbedder {
    calls: AtomicUsize,
}

#[async_trait]
impl muninn_core::ports::Embedder for FixedEmbedder {
    fn identity(&self) -> MemoryResult<EmbeddingIdentity> {
        EmbeddingIdentity::new("bridge:context:fixed-vector:v1", 1)
    }

    async fn embed(&self, _: &str) -> MemoryResult<Vec<f64>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(vec![1.0])
    }

    fn dimensions(&self) -> usize {
        1
    }
}

fn message(owner: &'static str, role: MessageRole, content: &str) -> (&'static str, Message) {
    (
        owner,
        Message {
            id: Uuid::new_v4(),
            role,
            content: content.to_owned(),
            conversation_id: Some(Uuid::new_v4()),
            created_at: Utc::now(),
            embedding: Some(vec![1.0]),
            metadata: Metadata::new(),
        },
    )
}

fn preference(
    owner: &'static str,
    category: &str,
    text: &str,
    context: Option<&str>,
) -> (&'static str, Preference) {
    (
        owner,
        Preference {
            id: Uuid::new_v4(),
            category: category.to_owned(),
            preference: text.to_owned(),
            context: context.map(str::to_owned),
            confidence: 1.0,
            embedding: Some(vec![1.0]),
            valid_from: None,
            valid_until: None,
            created_at: Utc::now(),
            metadata: Metadata::new(),
        },
    )
}

fn entity() -> Entity {
    Entity {
        id: Uuid::new_v4(),
        name: "Ada".to_owned(),
        canonical_name: Some("Ada Lovelace".to_owned()),
        entity_type: "PERSON".to_owned(),
        subtype: Some("SCIENTIST".to_owned()),
        description: Some("mathematician".to_owned()),
        confidence: 1.0,
        aliases: Vec::new(),
        embedding: Some(vec![1.0]),
        created_at: Utc::now(),
        updated_at: None,
        metadata: Metadata::new(),
    }
}

/// Records the scope each read received, so the test can prove the bridge did
/// not silently widen it.
struct ContextStore {
    messages: Vec<(&'static str, Message)>,
    preferences: Vec<(&'static str, Preference)>,
    entities: Vec<Entity>,
    traces: Vec<ScoredTrace>,
    reads: Mutex<Vec<(&'static str, Option<String>)>>,
}

impl ContextStore {
    /// Owner `alice` and a foreign owner `bob`; one entity and one trace.
    fn fixture() -> Self {
        Self {
            messages: vec![
                message("alice", MessageRole::User, "alice private note"),
                message("bob", MessageRole::User, "bob private note"),
            ],
            preferences: vec![
                preference("alice", "style", "concise", Some("technical answers")),
                preference("bob", "food", "tea", None),
            ],
            entities: vec![entity()],
            traces: vec![ScoredTrace {
                trace: ReasoningTrace {
                    id: Uuid::new_v4(),
                    session_id: "session".to_owned(),
                    task: "find Ada".to_owned(),
                    task_embedding: Some(vec![1.0]),
                    steps: Vec::new(),
                    outcome: Some("done".to_owned()),
                    success: Some(true),
                    started_at: Utc::now(),
                    completed_at: None,
                    metadata: Metadata::new(),
                },
                score: 1.0,
            }],
            reads: Mutex::new(Vec::new()),
        }
    }

    const fn empty() -> Self {
        Self {
            messages: Vec::new(),
            preferences: Vec::new(),
            entities: Vec::new(),
            traces: Vec::new(),
            reads: Mutex::new(Vec::new()),
        }
    }

    /// Only the requested owner's rows; the unowned namespace sees every row.
    fn visible_messages(&self, scope: &OwnerScope) -> Vec<Message> {
        self.messages
            .iter()
            .filter(|(owner, _)| scope.as_str().is_none_or(|scope| *owner == scope))
            .map(|(_, message)| message.clone())
            .collect()
    }

    fn record(&self, operation: &'static str, scope: Option<&str>) {
        self.reads
            .lock()
            .unwrap()
            .push((operation, scope.map(str::to_owned)));
    }

    fn reads(&self) -> Vec<(&'static str, Option<String>)> {
        self.reads.lock().unwrap().clone()
    }
}

async fn post(
    state: Arc<BridgeState>,
    method: &str,
    body: serde_json::Value,
) -> (StatusCode, String) {
    let response = router(state)
        .oneshot(
            Request::post(format!("/{method}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

fn bridge(store: &Arc<ContextStore>, embedder: &Arc<FixedEmbedder>) -> Arc<BridgeState> {
    Arc::new(BridgeState {
        memory: Arc::new(MemoryClient::new(
            MemoryPorts {
                short_term: store.clone(),
                long_term: store.clone(),
                reasoning: store.clone(),
                embedder: Some(embedder.clone()),
                cypher_query: None,
            },
            &MemoryOptions::default(),
        )),
        maintenance: None,
    })
}

#[tokio::test]
async fn context_document_reproduces_the_pinned_section_order_and_fields() {
    let store = Arc::new(ContextStore::fixture());
    let embedder = Arc::new(FixedEmbedder::default());
    let (status, body) = post(
        bridge(&store, &embedder),
        "get_context",
        json!({"query": "Ada", "user_identifier": "alice"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let document: String =
        serde_json::from_str(&body).expect("the bridge returns the document string");
    let expected = "\
## Conversation History

### Relevant Past Messages
- [user] alice private note (relevance: 1.00)

## Relevant Knowledge
### User Preferences
- [style] concise (context: technical answers)

### Relevant Entities
- Ada Lovelace (PERSON:SCIENTIST): mathematician

## Similar Past Tasks

**Task**: find Ada
- Similarity: 1.00
- Outcome: done
- Success: Yes";
    assert_eq!(document, expected);
}

#[tokio::test]
async fn context_of_an_empty_memory_is_the_pinned_empty_document() {
    let store = Arc::new(ContextStore::empty());
    let embedder = Arc::new(FixedEmbedder::default());
    let (status, body) = post(
        bridge(&store, &embedder),
        "get_context",
        json!({"query": "Ada", "max_items": 3}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, "\"\"");
}

#[tokio::test]
async fn context_omits_every_section_that_is_switched_off() {
    let store = Arc::new(ContextStore::fixture());
    let embedder = Arc::new(FixedEmbedder::default());
    let (status, body) = post(
        bridge(&store, &embedder),
        "get_context",
        json!({
            "query": "Ada",
            "user_identifier": "alice",
            "include_short_term": false,
            "include_long_term": false,
            "include_reasoning": false,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, "\"\"");
    assert!(
        store.reads().is_empty(),
        "no section may be read: {:?}",
        store.reads()
    );

    let (status, body) = post(
        bridge(&store, &embedder),
        "get_context",
        json!({"query": "Ada", "user_identifier": "alice", "include_reasoning": false}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let document: String = serde_json::from_str(&body).unwrap();
    assert!(document.contains("## Conversation History"), "{document}");
    assert!(document.contains("## Relevant Knowledge"), "{document}");
    assert!(!document.contains("## Similar Past Tasks"), "{document}");
}

#[tokio::test]
async fn context_never_returns_another_owners_rows() {
    let store = Arc::new(ContextStore::fixture());
    let embedder = Arc::new(FixedEmbedder::default());

    let (status, body) = post(
        bridge(&store, &embedder),
        "get_context",
        json!({"query": "Ada", "user_identifier": "alice"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let alice: String = serde_json::from_str(&body).unwrap();
    for foreign in ["bob private note", "- [food] tea"] {
        assert!(!alice.contains(foreign), "leaked {foreign:?}:\n{alice}");
    }
    assert!(alice.contains("alice private note"), "{alice}");

    let (status, body) = post(
        bridge(&store, &embedder),
        "get_context",
        json!({"query": "Ada", "user_identifier": "bob"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let bob: String = serde_json::from_str(&body).unwrap();
    assert!(bob.contains("bob private note"), "{bob}");
    assert!(bob.contains("- [food] tea"), "{bob}");
    for foreign in ["alice private note", "- [style] concise"] {
        assert!(!bob.contains(foreign), "leaked {foreign:?}:\n{bob}");
    }

    // The pinned fused client also reads similar traces; that store read is
    // unscoped because reasoning traces are global in Muninn's model.
    assert_eq!(
        store.reads(),
        vec![
            ("search_messages", Some("alice".to_owned())),
            ("search_preferences", Some("alice".to_owned())),
            ("search_entities", None),
            ("search_traces", None),
            ("search_messages", Some("bob".to_owned())),
            ("search_preferences", Some("bob".to_owned())),
            ("search_entities", None),
            ("search_traces", None),
        ],
        "every owner-scoped read must receive the requesting owner"
    );
}

#[tokio::test]
async fn malformed_context_arguments_are_rejected_before_any_read() {
    let store = Arc::new(ContextStore::fixture());
    let embedder = Arc::new(FixedEmbedder::default());
    for (arguments, message) in [
        (
            json!({"query": "Ada", "max_items": "ten"}),
            "'max_items' must be an integer",
        ),
        (
            json!({"query": "Ada", "include_reasoning": "yes"}),
            "'include_reasoning' must be a boolean",
        ),
        (
            json!({"query": "Ada", "user_identifier": 7}),
            "'user_identifier' must be a string",
        ),
        (json!({"max_items": 2}), "missing argument 'query'"),
        (json!({"query": 5}), "'query' must be a string"),
    ] {
        let (status, body) = post(bridge(&store, &embedder), "get_context", arguments).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body.contains(message), "{body} must name {message}");
        assert!(
            store.reads().is_empty(),
            "a rejected call must not read: {body}"
        );
        assert_eq!(embedder.calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn context_includes_the_recent_conversation_of_the_requested_session() {
    let mut store = ContextStore::fixture();
    store.messages = vec![
        message("alice", MessageRole::User, "alice private note"),
        message("alice", MessageRole::Assistant, "alice follow up"),
    ];
    let store = Arc::new(store);
    let embedder = Arc::new(FixedEmbedder::default());
    let (status, body) = post(
        bridge(&store, &embedder),
        "get_context",
        json!({"query": "Ada", "session_id": "s1", "user_identifier": "alice"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let document: String = serde_json::from_str(&body).unwrap();
    assert!(
        document.starts_with(
            "## Conversation History\n### Recent Conversation\n**user**: alice private note\n\
             **assistant**: alice follow up\n\n### Relevant Past Messages\n"
        ),
        "{document}"
    );
    assert_eq!(
        store.reads().first(),
        Some(&("get_conversation", Some("alice".to_owned())))
    );
}

#[async_trait]
impl ShortTermStore for ContextStore {
    async fn ensure_conversation(
        &self,
        _: &str,
        _: Option<Uuid>,
        _: &OwnerScope,
    ) -> MemoryResult<Uuid> {
        unimplemented!()
    }
    async fn find_by_operation_key(
        &self,
        _: Uuid,
        _: &OwnerScope,
        _: &str,
    ) -> MemoryResult<Option<KeyedMessage>> {
        unimplemented!()
    }
    async fn append_message(
        &self,
        _: Uuid,
        _: &OwnerScope,
        _: &MessageRecord,
    ) -> MemoryResult<Message> {
        unimplemented!()
    }
    async fn append_messages(
        &self,
        _: Uuid,
        _: &OwnerScope,
        _: &[MessageRecord],
    ) -> MemoryResult<Vec<Message>> {
        unimplemented!()
    }
    async fn get_conversation(
        &self,
        session_id: &str,
        _: Option<Uuid>,
        scope: &OwnerScope,
        _: Option<i64>,
    ) -> MemoryResult<Option<Conversation>> {
        self.record("get_conversation", scope.as_str());
        let messages = self.visible_messages(scope);
        if messages.is_empty() {
            return Ok(None);
        }
        let mut conversation = Conversation::empty(session_id);
        conversation.messages = messages;
        Ok(Some(conversation))
    }
    async fn get_conversation_by_id(
        &self,
        _: Uuid,
        _: &OwnerScope,
    ) -> MemoryResult<Option<Conversation>> {
        unimplemented!()
    }
    async fn list_conversations(&self, _: &OwnerScope, _: i64) -> MemoryResult<Vec<Conversation>> {
        unimplemented!()
    }
    async fn search_messages_with_identity(
        &self,
        _: &[f64],
        _: &EmbeddingIdentity,
        _: Option<&str>,
        scope: &OwnerScope,
        limit: i64,
        threshold: f64,
        _: &Metadata,
    ) -> MemoryResult<Vec<ScoredMessage>> {
        self.record("search_messages", scope.as_str());
        let hits = self
            .visible_messages(scope)
            .into_iter()
            .filter(|message| message.embedding.is_some())
            .take(usize::try_from(limit).unwrap_or(0))
            .map(|message| ScoredMessage {
                message,
                score: 1.0,
            })
            .filter(|hit| hit.score >= threshold)
            .collect();
        Ok(hits)
    }
    async fn search_messages(
        &self,
        _: &[f64],
        _: Option<&str>,
        _: &OwnerScope,
        _: i64,
        _: f64,
        _: &Metadata,
    ) -> MemoryResult<Vec<ScoredMessage>> {
        unimplemented!()
    }
    async fn list_sessions(
        &self,
        _: &SessionQuery,
        _: &OwnerScope,
    ) -> MemoryResult<Vec<SessionInfo>> {
        unimplemented!()
    }
    async fn delete_message(&self, _: Uuid, _: &OwnerScope, _: bool) -> MemoryResult<bool> {
        unimplemented!()
    }
    async fn clear_session(&self, _: &str, _: &OwnerScope) -> MemoryResult<()> {
        unimplemented!()
    }
    async fn delete_conversation(&self, _: Uuid, _: &OwnerScope) -> MemoryResult<()> {
        unimplemented!()
    }
}

#[async_trait]
impl LongTermStore for ContextStore {
    async fn upsert_entity(&self, _: &Entity) -> MemoryResult<Entity> {
        unimplemented!()
    }
    async fn get_entity(&self, _: Uuid) -> MemoryResult<Option<Entity>> {
        unimplemented!()
    }
    async fn get_entity_by_name(&self, _: &str) -> MemoryResult<Option<Entity>> {
        unimplemented!()
    }
    async fn search_entities(
        &self,
        _: &[f64],
        limit: i64,
        threshold: f64,
    ) -> MemoryResult<Vec<Scored<Entity>>> {
        self.record("search_entities", None);
        let _ = threshold;
        Ok(self
            .entities
            .iter()
            .take(usize::try_from(limit).unwrap_or(0))
            .map(|entity| Scored {
                item: entity.clone(),
                score: 1.0,
            })
            .collect())
    }
    async fn list_entities(&self, _: Option<&str>, _: i64) -> MemoryResult<Vec<Entity>> {
        unimplemented!()
    }
    async fn update_entity(&self, _: Uuid, _: &EntityUpdate) -> MemoryResult<Option<Entity>> {
        unimplemented!()
    }
    async fn delete_entity(&self, _: Uuid) -> MemoryResult<bool> {
        unimplemented!()
    }
    async fn related_entities(&self, _: Uuid, _: u8) -> MemoryResult<Vec<RelatedEntity>> {
        unimplemented!()
    }
    async fn upsert_relationship(&self, _: &Relationship) -> MemoryResult<Option<Relationship>> {
        unimplemented!()
    }
    async fn merge_entities(&self, _: Uuid, _: Uuid) -> MemoryResult<Option<(Entity, Entity)>> {
        unimplemented!()
    }
    async fn link_same_as(&self, _: Uuid, _: Uuid, _: f64, _: &str, _: &str) -> MemoryResult<()> {
        unimplemented!()
    }
    async fn set_entity_feedback(&self, _: Uuid, _: f64, _: bool) -> MemoryResult<bool> {
        unimplemented!()
    }
    async fn link_message_to_entity(&self, _: Uuid, _: Uuid, _: f64) -> MemoryResult<bool> {
        unimplemented!()
    }
    async fn entity_mentions(&self, _: Uuid) -> MemoryResult<Option<Vec<EntityMention>>> {
        unimplemented!()
    }
    async fn entity_graph(&self, _: i64) -> MemoryResult<EntityGraph> {
        unimplemented!()
    }
    async fn create_preference(&self, _: &Preference) -> MemoryResult<Preference> {
        unimplemented!()
    }
    async fn find_duplicate_preference(
        &self,
        _: &[f64],
        _: &str,
        _: f64,
        _: Option<&str>,
    ) -> MemoryResult<Option<Preference>> {
        unimplemented!()
    }
    async fn link_user_to_preference(&self, _: &str, _: Uuid) -> MemoryResult<()> {
        unimplemented!()
    }
    async fn link_preference_to_entity(&self, _: Uuid, _: Uuid) -> MemoryResult<()> {
        unimplemented!()
    }
    async fn supersede_preference(&self, _: Uuid, _: Uuid) -> MemoryResult<()> {
        unimplemented!()
    }
    async fn preferences_for(&self, _: &str, _: bool) -> MemoryResult<Vec<Preference>> {
        unimplemented!()
    }
    async fn search_preferences(
        &self,
        _: &[f64],
        limit: i64,
        threshold: f64,
        user_identifier: Option<&str>,
    ) -> MemoryResult<Vec<Scored<Preference>>> {
        self.record("search_preferences", user_identifier);
        Ok(self
            .preferences
            .iter()
            .filter(|(owner, _)| user_identifier.is_none_or(|scope| *owner == scope))
            .take(usize::try_from(limit).unwrap_or(0))
            .map(|(_, preference)| Scored {
                item: preference.clone(),
                score: 1.0,
            })
            .filter(|hit| hit.score >= threshold)
            .collect())
    }
    async fn preferences_by_category(
        &self,
        _: &str,
        _: i64,
        _: Option<&str>,
    ) -> MemoryResult<Vec<Preference>> {
        unimplemented!()
    }
    async fn create_fact(&self, _: &Fact) -> MemoryResult<Fact> {
        unimplemented!()
    }
    async fn find_duplicate_fact(
        &self,
        _: &[f64],
        _: &str,
        _: &str,
        _: f64,
    ) -> MemoryResult<Option<Fact>> {
        unimplemented!()
    }
    async fn update_fact_confidence(&self, _: Uuid, _: f64, _: &str) -> MemoryResult<()> {
        unimplemented!()
    }
    async fn search_facts(&self, _: &[f64], _: i64, _: f64) -> MemoryResult<Vec<Scored<Fact>>> {
        unimplemented!()
    }
    async fn facts_by_subject(&self, _: &str, _: i64) -> MemoryResult<Vec<Fact>> {
        unimplemented!()
    }
}

#[async_trait]
impl ReasoningStore for ContextStore {
    async fn create_trace(
        &self,
        _: &TraceRecord,
        _: &OwnerScope,
        _: Option<Uuid>,
    ) -> MemoryResult<ReasoningTrace> {
        unimplemented!()
    }
    async fn validate_message_scope(&self, _: Uuid, _: &OwnerScope) -> MemoryResult<()> {
        unimplemented!()
    }
    async fn add_conversation_step(
        &self,
        _: Uuid,
        _: Uuid,
        _: &OwnerScope,
        _: &StepRecord,
    ) -> MemoryResult<ReasoningStep> {
        unimplemented!()
    }
    async fn link_user_to_trace(&self, _: &str, _: Uuid) -> MemoryResult<()> {
        unimplemented!()
    }
    async fn add_step(&self, _: Uuid, _: &StepRecord) -> MemoryResult<ReasoningStep> {
        unimplemented!()
    }
    async fn set_step_observation(&self, _: Uuid, _: &str) -> MemoryResult<()> {
        unimplemented!()
    }
    async fn steps_without_embedding(&self, _: Uuid) -> MemoryResult<Vec<ReasoningStep>> {
        unimplemented!()
    }
    async fn set_step_embedding(&self, _: Uuid, _: &[f64]) -> MemoryResult<()> {
        unimplemented!()
    }
    async fn record_tool_call(
        &self,
        _: &ToolCall,
        _: &OwnerScope,
        _: Option<Uuid>,
    ) -> MemoryResult<()> {
        unimplemented!()
    }
    async fn complete_trace(
        &self,
        _: Uuid,
        _: Option<&str>,
        _: Option<bool>,
    ) -> MemoryResult<Option<ReasoningTrace>> {
        unimplemented!()
    }
    async fn get_trace_with_steps(&self, _: Uuid) -> MemoryResult<Option<ReasoningTrace>> {
        unimplemented!()
    }
    async fn list_traces(&self, _: &TraceQuery) -> MemoryResult<Vec<ReasoningTrace>> {
        unimplemented!()
    }
    async fn tool_stats(&self, _: Option<&str>) -> MemoryResult<Vec<ToolStats>> {
        unimplemented!()
    }
    async fn search_traces(
        &self,
        _: &[f64],
        limit: i64,
        threshold: f64,
        _: bool,
    ) -> MemoryResult<Vec<ScoredTrace>> {
        self.record("search_traces", None);
        Ok(self
            .traces
            .iter()
            .take(usize::try_from(limit).unwrap_or(0))
            .filter(|hit| hit.score >= threshold)
            .cloned()
            .collect())
    }
    async fn search_steps(
        &self,
        _: &[f64],
        _: i64,
        _: f64,
        _: bool,
    ) -> MemoryResult<Vec<ReasoningStepWithContext>> {
        unimplemented!()
    }
    async fn link_trace_to_message(&self, _: Uuid, _: Uuid, _: &OwnerScope) -> MemoryResult<bool> {
        unimplemented!()
    }
    async fn link_conversation_to_trace(
        &self,
        _: Uuid,
        _: Uuid,
        _: &OwnerScope,
    ) -> MemoryResult<bool> {
        unimplemented!()
    }
    async fn link_tool_call_to_message(
        &self,
        _: Uuid,
        _: Uuid,
        _: &OwnerScope,
    ) -> MemoryResult<bool> {
        unimplemented!()
    }
    async fn record_touched(&self, _: Uuid, _: &EntityRef) -> MemoryResult<bool> {
        unimplemented!()
    }
    async fn open_conversation_trace(&self, _: Uuid, _: &OwnerScope) -> MemoryResult<Option<Uuid>> {
        unimplemented!()
    }
    async fn ensure_conversation_trace(
        &self,
        _: Uuid,
        _: &TraceRecord,
        _: &OwnerScope,
    ) -> MemoryResult<Uuid> {
        unimplemented!()
    }
    async fn conversation_steps(
        &self,
        _: Uuid,
        _: &OwnerScope,
    ) -> MemoryResult<Option<Vec<ReasoningStep>>> {
        unimplemented!()
    }
    async fn step_context(&self, _: Uuid, _: &OwnerScope) -> MemoryResult<Option<StepContext>> {
        unimplemented!()
    }
    async fn entity_provenance(
        &self,
        _: &str,
        _: &OwnerScope,
    ) -> MemoryResult<Vec<ConversationStep>> {
        unimplemented!()
    }
}
