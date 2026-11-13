//! Router-level behavior that does not need a graph database.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use muninn_application::{MemoryClient, MemoryOptions, MemoryPorts};
use muninn_bridge::{router, BridgeState};
use muninn_core::model::{
    Conversation, Entity, EntityGraph, EntityMention, EntityRef, Fact, Message, Metadata,
    Preference, ReasoningStep, ReasoningStepWithContext, ReasoningTrace, Relationship, SessionInfo,
    ToolCall, ToolStats,
};
use muninn_core::ports::{
    ConversationStep, EmbeddingIdentity, EntityUpdate, GraphMaintenance, KeyedMessage,
    LongTermStore, MessageRecord, OwnerScope, ReasoningStore, RelatedEntity, Scored, ScoredMessage,
    ScoredTrace, SessionQuery, ShortTermStore, StepContext, StepRecord, TraceQuery, TraceRecord,
};
use muninn_core::query::{CypherQuery, CypherResult};
use muninn_core::{MemoryError, MemoryResult};
use tower::ServiceExt;
use uuid::Uuid;

/// Store that must never be reached by these tests.
enum UnreachableStore {
    Never,
    List {
        owner: OwnerScope,
    },
    Add {
        calls: Arc<Mutex<AddCalls>>,
        conversation_id: Uuid,
    },
    Search {
        owner: Option<String>,
        session: Option<String>,
    },
}

#[derive(Default)]
struct AddCalls {
    ensured: Vec<(String, Option<Uuid>, Option<String>)>,
    appended: Vec<(Uuid, OwnerScope, MessageRecord)>,
}

struct UnreachableEmbedder;
#[async_trait]
impl muninn_core::ports::Embedder for UnreachableEmbedder {
    async fn embed(&self, _: &str) -> MemoryResult<Vec<f64>> {
        unreachable!()
    }
    async fn embed_batch(&self, _: &[String]) -> MemoryResult<Vec<Vec<f64>>> {
        unreachable!()
    }
    fn dimensions(&self) -> usize {
        1
    }
}

#[derive(Default)]
struct RecordingEmbedder {
    identity_calls: AtomicUsize,
    embed_calls: AtomicUsize,
    batch_calls: AtomicUsize,
    dimensions_calls: AtomicUsize,
}

#[async_trait]
impl muninn_core::ports::Embedder for RecordingEmbedder {
    fn identity(&self) -> MemoryResult<EmbeddingIdentity> {
        self.identity_calls.fetch_add(1, Ordering::SeqCst);
        EmbeddingIdentity::new("bridge:test:recording:v1", 1)
    }

    async fn embed(&self, text: &str) -> MemoryResult<Vec<f64>> {
        assert_eq!(text, "hello");
        self.embed_calls.fetch_add(1, Ordering::SeqCst);
        Ok(vec![1.0])
    }

    async fn embed_batch(&self, _: &[String]) -> MemoryResult<Vec<Vec<f64>>> {
        self.batch_calls.fetch_add(1, Ordering::SeqCst);
        unreachable!("add_message must not use batch embedding")
    }

    fn dimensions(&self) -> usize {
        self.dimensions_calls.fetch_add(1, Ordering::SeqCst);
        1
    }
}

struct SearchEmbedder;

#[async_trait]
impl muninn_core::ports::Embedder for SearchEmbedder {
    fn identity(&self) -> MemoryResult<EmbeddingIdentity> {
        EmbeddingIdentity::new("bridge:test:fixed-vector:v1", 1)
    }

    async fn embed(&self, _: &str) -> MemoryResult<Vec<f64>> {
        Ok(vec![1.0])
    }
    async fn embed_batch(&self, _: &[String]) -> MemoryResult<Vec<Vec<f64>>> {
        unreachable!()
    }
    fn dimensions(&self) -> usize {
        1
    }
}

struct FailOnSecondEmbedder(Arc<AtomicUsize>);

#[async_trait]
impl muninn_core::ports::Embedder for FailOnSecondEmbedder {
    fn identity(&self) -> MemoryResult<EmbeddingIdentity> {
        EmbeddingIdentity::new("bridge:test:fixed-vector:v1", 1)
    }

    async fn embed(&self, _: &str) -> MemoryResult<Vec<f64>> {
        if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
            Ok(vec![1.0])
        } else {
            Err(MemoryError::provider("embedding provider unavailable"))
        }
    }

    async fn embed_batch(&self, _: &[String]) -> MemoryResult<Vec<Vec<f64>>> {
        unreachable!()
    }

    fn dimensions(&self) -> usize {
        1
    }
}

struct RecordingCypherQuery {
    called: AtomicBool,
}

#[async_trait]
impl CypherQuery for RecordingCypherQuery {
    async fn run_read_only(
        &self,
        _: &str,
        _: &serde_json::Map<String, serde_json::Value>,
    ) -> MemoryResult<CypherResult> {
        self.called.store(true, Ordering::SeqCst);
        Ok(CypherResult {
            columns: vec!["answer".to_owned()],
            rows: vec![vec![serde_json::json!(42)]],
        })
    }
}

#[async_trait]
impl ShortTermStore for UnreachableStore {
    async fn ensure_conversation(
        &self,
        session: &str,
        requested_id: Option<Uuid>,
        owner: &OwnerScope,
    ) -> MemoryResult<Uuid> {
        let Self::Add {
            calls,
            conversation_id,
        } = self
        else {
            unreachable!()
        };
        calls.lock().unwrap().ensured.push((
            session.to_owned(),
            requested_id,
            owner.as_str().map(str::to_owned),
        ));
        Ok(*conversation_id)
    }
    async fn find_by_operation_key(
        &self,
        id: Uuid,
        scope: &OwnerScope,
        operation_key: &str,
    ) -> MemoryResult<Option<KeyedMessage>> {
        let Self::Add {
            calls,
            conversation_id,
        } = self
        else {
            unreachable!()
        };
        assert_eq!(id, *conversation_id);
        let record = {
            let calls = calls.lock().unwrap();
            calls
                .appended
                .iter()
                .find(|(stored_conversation, stored_scope, record)| {
                    *stored_conversation == id
                        && stored_scope == scope
                        && record.operation_key.as_deref() == Some(operation_key)
                })
                .map(|(_, _, record)| record.clone())
        };
        let Some(record) = record else {
            return Ok(None);
        };
        Ok(Some(KeyedMessage {
            message: Message {
                id: record.id,
                role: record.role.parse()?,
                conversation_id: Some(id),
                content: record.content.clone(),
                created_at: record.timestamp.unwrap_or_else(chrono::Utc::now),
                metadata: record.metadata.clone(),
                embedding: record.embedding.clone(),
            },
            operation_payload: record.operation_payload,
        }))
    }
    async fn append_message(
        &self,
        id: Uuid,
        scope: &OwnerScope,
        record: &MessageRecord,
    ) -> MemoryResult<Message> {
        let Self::Add {
            calls,
            conversation_id,
        } = self
        else {
            unreachable!()
        };
        assert_eq!(id, *conversation_id);
        let mut record = record.clone();
        record.timestamp = Some(chrono::Utc::now());
        calls
            .lock()
            .unwrap()
            .appended
            .push((id, scope.clone(), record.clone()));
        Ok(Message {
            id: record.id,
            role: muninn_core::model::MessageRole::User,
            conversation_id: Some(id),
            content: record.content.clone(),
            created_at: record.timestamp.unwrap(),
            metadata: record.metadata,
            embedding: record.embedding,
        })
    }
    async fn append_messages(
        &self,
        _: Uuid,
        _: &OwnerScope,
        _: &[MessageRecord],
    ) -> MemoryResult<Vec<Message>> {
        unreachable!()
    }
    async fn get_conversation(
        &self,
        session: &str,
        _: Option<Uuid>,
        scope: &OwnerScope,
        _: Option<i64>,
    ) -> MemoryResult<Option<Conversation>> {
        let Self::Add {
            calls,
            conversation_id,
        } = self
        else {
            unreachable!()
        };
        let calls = calls.lock().unwrap();
        assert!(calls.appended.iter().all(|(_, stored, _)| stored == scope));
        Ok(if calls.ensured.is_empty() {
            None
        } else {
            let mut conversation = Conversation::empty(session);
            conversation.id = *conversation_id;
            Some(conversation)
        })
    }
    async fn get_conversation_by_id(
        &self,
        _: Uuid,
        _: &OwnerScope,
    ) -> MemoryResult<Option<Conversation>> {
        unreachable!()
    }
    async fn list_conversations(
        &self,
        scope: &OwnerScope,
        _: i64,
    ) -> MemoryResult<Vec<Conversation>> {
        let Self::List { owner } = self else {
            unreachable!()
        };
        assert_eq!(scope, owner);
        Ok(Vec::new())
    }
    async fn search_messages(
        &self,
        embedding: &[f64],
        session_id: Option<&str>,
        user_identifier: &OwnerScope,
        _: i64,
        _: f64,
        _: &Metadata,
    ) -> MemoryResult<Vec<ScoredMessage>> {
        let _ = (embedding, session_id, user_identifier);
        panic!("short-term application must use identity-aware search")
    }

    async fn search_messages_with_identity(
        &self,
        embedding: &[f64],
        identity: &EmbeddingIdentity,
        session_id: Option<&str>,
        user_identifier: &OwnerScope,
        _: i64,
        _: f64,
        _: &Metadata,
    ) -> MemoryResult<Vec<ScoredMessage>> {
        assert_eq!(identity.key(), "bridge:test:fixed-vector:v1");
        assert_eq!(identity.dimensions(), 1);
        let Self::Search { owner, session } = self else {
            unreachable!()
        };
        assert_eq!(embedding, &[1.0]);
        assert_eq!(user_identifier.as_str(), owner.as_deref());
        assert_eq!(session_id, session.as_deref());
        Ok(Vec::new())
    }
    async fn list_sessions(
        &self,
        _: &SessionQuery,
        _: &OwnerScope,
    ) -> MemoryResult<Vec<SessionInfo>> {
        unreachable!()
    }
    async fn delete_message(&self, _: Uuid, _: &OwnerScope, _: bool) -> MemoryResult<bool> {
        unreachable!()
    }
    async fn clear_session(&self, _: &str, _: &OwnerScope) -> MemoryResult<()> {
        unreachable!()
    }
    async fn delete_conversation(&self, _: Uuid, _: &OwnerScope) -> MemoryResult<()> {
        unreachable!()
    }
}

#[derive(Default)]
struct RecordingMaintenance(AtomicBool);

#[async_trait]
impl GraphMaintenance for RecordingMaintenance {
    async fn clear_all_data(&self) -> MemoryResult<()> {
        self.0.store(true, Ordering::SeqCst);
        Ok(())
    }
}

/// Long-term store that must never be reached by these tests.
struct UnreachableLongTerm;

#[async_trait]
impl LongTermStore for UnreachableLongTerm {
    async fn upsert_entity(&self, _: &Entity) -> MemoryResult<Entity> {
        unreachable!()
    }
    async fn get_entity(&self, _: Uuid) -> MemoryResult<Option<Entity>> {
        unreachable!()
    }
    async fn get_entity_by_name(&self, _: &str) -> MemoryResult<Option<Entity>> {
        unreachable!()
    }
    async fn search_entities(
        &self,
        _: &[f64],
        _: i64,
        _: f64,
    ) -> MemoryResult<Vec<Scored<Entity>>> {
        unreachable!()
    }
    async fn list_entities(&self, _: Option<&str>, _: i64) -> MemoryResult<Vec<Entity>> {
        unreachable!()
    }
    async fn update_entity(&self, _: Uuid, _: &EntityUpdate) -> MemoryResult<Option<Entity>> {
        unreachable!()
    }
    async fn delete_entity(&self, _: Uuid) -> MemoryResult<bool> {
        unreachable!()
    }
    async fn related_entities(&self, _: Uuid, _: u8) -> MemoryResult<Vec<RelatedEntity>> {
        unreachable!()
    }
    async fn upsert_relationship(&self, _: &Relationship) -> MemoryResult<Option<Relationship>> {
        unreachable!()
    }
    async fn merge_entities(&self, _: Uuid, _: Uuid) -> MemoryResult<Option<(Entity, Entity)>> {
        unreachable!()
    }
    async fn link_same_as(&self, _: Uuid, _: Uuid, _: f64, _: &str, _: &str) -> MemoryResult<()> {
        unreachable!()
    }
    async fn set_entity_feedback(&self, _: Uuid, _: f64, _: bool) -> MemoryResult<bool> {
        unreachable!()
    }
    async fn link_message_to_entity(&self, _: Uuid, _: Uuid, _: f64) -> MemoryResult<bool> {
        unreachable!()
    }
    async fn entity_mentions(&self, _: Uuid) -> MemoryResult<Option<Vec<EntityMention>>> {
        unreachable!()
    }
    async fn entity_graph(&self, _: i64) -> MemoryResult<EntityGraph> {
        unreachable!()
    }
    async fn create_preference(&self, _: &Preference) -> MemoryResult<Preference> {
        unreachable!()
    }
    async fn find_duplicate_preference(
        &self,
        _: &[f64],
        _: &str,
        _: f64,
        _: Option<&str>,
    ) -> MemoryResult<Option<Preference>> {
        unreachable!()
    }
    async fn link_user_to_preference(&self, _: &str, _: Uuid) -> MemoryResult<()> {
        unreachable!()
    }
    async fn link_preference_to_entity(&self, _: Uuid, _: Uuid) -> MemoryResult<()> {
        unreachable!()
    }
    async fn supersede_preference(&self, _: Uuid, _: Uuid) -> MemoryResult<()> {
        unreachable!()
    }
    async fn preferences_for(&self, _: &str, _: bool) -> MemoryResult<Vec<Preference>> {
        unreachable!()
    }
    async fn search_preferences(
        &self,
        _: &[f64],
        _: i64,
        _: f64,
        _: Option<&str>,
    ) -> MemoryResult<Vec<Scored<Preference>>> {
        unreachable!()
    }
    async fn preferences_by_category(
        &self,
        _: &str,
        _: i64,
        _: Option<&str>,
    ) -> MemoryResult<Vec<Preference>> {
        unreachable!()
    }
    async fn create_fact(&self, _: &Fact) -> MemoryResult<Fact> {
        unreachable!()
    }
    async fn find_duplicate_fact(
        &self,
        _: &[f64],
        _: &str,
        _: &str,
        _: f64,
    ) -> MemoryResult<Option<Fact>> {
        unreachable!()
    }
    async fn update_fact_confidence(&self, _: Uuid, _: f64, _: &str) -> MemoryResult<()> {
        unreachable!()
    }
    async fn search_facts(&self, _: &[f64], _: i64, _: f64) -> MemoryResult<Vec<Scored<Fact>>> {
        unreachable!()
    }
    async fn facts_by_subject(&self, _: &str, _: i64) -> MemoryResult<Vec<Fact>> {
        unreachable!()
    }
}

/// Reasoning store that must never be reached by these tests.
struct UnreachableReasoning;

#[async_trait]
impl ReasoningStore for UnreachableReasoning {
    async fn validate_message_scope(&self, _: Uuid, _: &OwnerScope) -> MemoryResult<()> {
        panic!("unexpected dependency")
    }
    async fn add_conversation_step(
        &self,
        _: Uuid,
        _: Uuid,
        _: &OwnerScope,
        _: &StepRecord,
    ) -> MemoryResult<ReasoningStep> {
        panic!("unexpected dependency")
    }

    async fn create_trace(
        &self,
        _: &TraceRecord,
        _: &OwnerScope,
        _: Option<Uuid>,
    ) -> MemoryResult<ReasoningTrace> {
        unreachable!()
    }
    async fn link_user_to_trace(&self, _: &str, _: Uuid) -> MemoryResult<()> {
        unreachable!()
    }
    async fn add_step(&self, _: Uuid, _: &StepRecord) -> MemoryResult<ReasoningStep> {
        unreachable!()
    }
    async fn set_step_observation(&self, _: Uuid, _: &str) -> MemoryResult<()> {
        unreachable!()
    }
    async fn steps_without_embedding(&self, _: Uuid) -> MemoryResult<Vec<ReasoningStep>> {
        unreachable!()
    }
    async fn set_step_embedding(&self, _: Uuid, _: &[f64]) -> MemoryResult<()> {
        unreachable!()
    }
    async fn record_tool_call(
        &self,
        _: &ToolCall,
        _: &OwnerScope,
        _: Option<Uuid>,
    ) -> MemoryResult<()> {
        unreachable!()
    }
    async fn complete_trace(
        &self,
        _: Uuid,
        _: Option<&str>,
        _: Option<bool>,
    ) -> MemoryResult<Option<ReasoningTrace>> {
        unreachable!()
    }
    async fn get_trace_with_steps(&self, _: Uuid) -> MemoryResult<Option<ReasoningTrace>> {
        unreachable!()
    }
    async fn list_traces(&self, _: &TraceQuery) -> MemoryResult<Vec<ReasoningTrace>> {
        unreachable!()
    }
    async fn tool_stats(&self, _: Option<&str>) -> MemoryResult<Vec<ToolStats>> {
        unreachable!()
    }
    async fn search_traces(
        &self,
        _: &[f64],
        _: i64,
        _: f64,
        _: bool,
    ) -> MemoryResult<Vec<ScoredTrace>> {
        unreachable!()
    }
    async fn search_steps(
        &self,
        _: &[f64],
        _: i64,
        _: f64,
        _: bool,
    ) -> MemoryResult<Vec<ReasoningStepWithContext>> {
        unreachable!()
    }
    async fn link_trace_to_message(&self, _: Uuid, _: Uuid, _: &OwnerScope) -> MemoryResult<bool> {
        unreachable!()
    }
    async fn link_conversation_to_trace(
        &self,
        _: Uuid,
        _: Uuid,
        _: &OwnerScope,
    ) -> MemoryResult<bool> {
        unreachable!()
    }
    async fn link_tool_call_to_message(
        &self,
        _: Uuid,
        _: Uuid,
        _: &OwnerScope,
    ) -> MemoryResult<bool> {
        unreachable!()
    }
    async fn record_touched(&self, _: Uuid, _: &EntityRef) -> MemoryResult<bool> {
        unreachable!()
    }
    async fn open_conversation_trace(&self, _: Uuid, _: &OwnerScope) -> MemoryResult<Option<Uuid>> {
        unreachable!()
    }
    async fn ensure_conversation_trace(
        &self,
        _: Uuid,
        _: &TraceRecord,
        _: &OwnerScope,
    ) -> MemoryResult<Uuid> {
        unreachable!()
    }
    async fn conversation_steps(
        &self,
        _: Uuid,
        _: &OwnerScope,
    ) -> MemoryResult<Option<Vec<ReasoningStep>>> {
        unreachable!()
    }
    async fn step_context(&self, _: Uuid, _: &OwnerScope) -> MemoryResult<Option<StepContext>> {
        unreachable!()
    }
    async fn entity_provenance(
        &self,
        _: &str,
        _: &OwnerScope,
    ) -> MemoryResult<Vec<ConversationStep>> {
        unreachable!()
    }
}

fn state(
    maintenance: Option<Arc<dyn GraphMaintenance>>,
    cypher_query: Option<Arc<dyn CypherQuery>>,
) -> Arc<BridgeState> {
    let memory = MemoryClient::new(
        MemoryPorts {
            short_term: Arc::new(UnreachableStore::Never),
            long_term: Arc::new(UnreachableLongTerm),
            reasoning: Arc::new(UnreachableReasoning),
            embedder: None,
            cypher_query,
        },
        &MemoryOptions::default(),
    );
    Arc::new(BridgeState {
        memory: Arc::new(memory),
        maintenance,
    })
}

async fn post(state: Arc<BridgeState>, method: &str, body: &str) -> StatusCode {
    request(state, method, body).await.status()
}

async fn request(state: Arc<BridgeState>, method: &str, body: &str) -> axum::http::Response<Body> {
    router(state)
        .oneshot(
            Request::post(format!("/{method}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn native_create_requires_session_without_changing_hosted_contract() {
    assert_eq!(
        post(state(None, None), "create_session_conversation", "{}").await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        post(
            state(None, None),
            "create_conversation",
            r#"{"session_id":"native"}"#
        )
        .await,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn cypher_query_reaches_read_only_port() {
    let port = Arc::new(RecordingCypherQuery {
        called: AtomicBool::new(false),
    });
    let response = request(
        state(None, Some(port.clone())),
        "cypher_query",
        r#"{"cypher":"RETURN 42 AS answer","params":{}}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
        serde_json::json!({"columns":["answer"],"rows":[[42]]})
    );
    assert!(port.called.load(Ordering::SeqCst));
}

#[tokio::test]
async fn cypher_query_rejects_writes_before_reaching_port() {
    let port = Arc::new(RecordingCypherQuery {
        called: AtomicBool::new(false),
    });
    assert_eq!(
        post(
            state(None, Some(port.clone())),
            "cypher_query",
            r#"{"cypher":"CREATE (n:Unsafe)","params":{}}"#,
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    assert!(!port.called.load(Ordering::SeqCst));
}

#[tokio::test]
async fn clear_all_data_is_refused_unless_explicitly_enabled() {
    assert_eq!(
        post(state(None, None), "clear_all_data", "{}").await,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn clear_all_data_wipes_when_enabled() {
    let maintenance = Arc::new(RecordingMaintenance::default());
    let status = post(
        state(Some(maintenance.clone()), None),
        "clear_all_data",
        "{}",
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(maintenance.0.load(Ordering::SeqCst));
}

#[tokio::test]
async fn unknown_methods_are_rejected() {
    assert_eq!(
        post(state(None, None), "drop_database", "{}").await,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn missing_arguments_are_bad_requests() {
    assert_eq!(
        post(state(None, None), "add_message", "{}").await,
        StatusCode::BAD_REQUEST
    );
}

fn add_state(calls: Arc<Mutex<AddCalls>>, embedder: Arc<RecordingEmbedder>) -> Arc<BridgeState> {
    Arc::new(BridgeState {
        memory: Arc::new(MemoryClient::new(
            MemoryPorts {
                short_term: Arc::new(UnreachableStore::Add {
                    calls,
                    conversation_id: Uuid::new_v4(),
                }),
                long_term: Arc::new(UnreachableLongTerm),
                reasoning: Arc::new(UnreachableReasoning),
                embedder: Some(embedder),
                cypher_query: None,
            },
            &MemoryOptions::default(),
        )),
        maintenance: None,
    })
}

#[tokio::test]
async fn add_message_respects_generate_embedding() {
    for (flag, expected_calls) in [
        (Some(serde_json::json!(false)), 0),
        (Some(serde_json::json!(true)), 1),
        (None, 1),
        (Some(serde_json::Value::Null), 1),
    ] {
        let calls = Arc::new(Mutex::new(AddCalls::default()));
        let embedder = Arc::new(RecordingEmbedder::default());
        let state = add_state(calls.clone(), embedder.clone());
        let mut args = serde_json::json!({
            "session_id": "session-a", "role": "user", "content": "hello"
        });
        if let Some(flag) = flag {
            args["generate_embedding"] = flag;
        }
        let response = request(state, "add_message", &args.to_string()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            embedder.identity_calls.load(Ordering::SeqCst),
            expected_calls
        );
        assert_eq!(embedder.embed_calls.load(Ordering::SeqCst), expected_calls);
        assert_eq!(embedder.batch_calls.load(Ordering::SeqCst), 0);
        if expected_calls == 0 {
            assert_eq!(embedder.dimensions_calls.load(Ordering::SeqCst), 0);
        }
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let calls = calls.lock().unwrap();
        assert_eq!(calls.ensured.len(), 1);
        assert_eq!(calls.appended.len(), 1);
        let expected_embedding = (expected_calls == 1).then(|| vec![1.0]);
        assert_eq!(calls.appended[0].2.embedding, expected_embedding);
        assert_eq!(body["embedding"], serde_json::json!(expected_embedding));
    }
}

#[tokio::test]
async fn add_message_rejects_invalid_generate_embedding_before_persistence() {
    for invalid in [
        serde_json::json!("false"),
        serde_json::json!(0),
        serde_json::json!([]),
        serde_json::json!({}),
    ] {
        let calls = Arc::new(Mutex::new(AddCalls::default()));
        let embedder = Arc::new(RecordingEmbedder::default());
        let state = add_state(calls.clone(), embedder.clone());
        let args = serde_json::json!({
            "session_id": "session-a", "role": "user", "content": "hello",
            "generate_embedding": invalid
        });
        let response = request(state, "add_message", &args.to_string()).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["error"], "'generate_embedding' must be a boolean");
        assert_eq!(embedder.identity_calls.load(Ordering::SeqCst), 0);
        assert_eq!(embedder.embed_calls.load(Ordering::SeqCst), 0);
        assert_eq!(embedder.batch_calls.load(Ordering::SeqCst), 0);
        assert_eq!(embedder.dimensions_calls.load(Ordering::SeqCst), 0);
        let calls = calls.lock().unwrap();
        assert!(calls.ensured.is_empty());
        assert!(calls.appended.is_empty());
        drop(calls);
    }
}

#[tokio::test]
async fn add_message_propagates_owner_and_preserves_absent_owner() {
    for (multi_tenant, owner) in [
        (false, Some("owner-a")),
        (true, Some("owner-a")),
        (false, None),
    ] {
        let calls = Arc::new(Mutex::new(AddCalls::default()));
        let conversation_id = Uuid::new_v4();
        let memory = Arc::new(MemoryClient::new(
            MemoryPorts {
                short_term: Arc::new(UnreachableStore::Add {
                    calls: calls.clone(),
                    conversation_id,
                }),
                long_term: Arc::new(UnreachableLongTerm),
                reasoning: Arc::new(UnreachableReasoning),
                embedder: None,
                cypher_query: None,
            },
            &MemoryOptions { multi_tenant },
        ));
        let state = Arc::new(BridgeState {
            memory,
            maintenance: None,
        });
        let mut args = serde_json::json!({
            "session_id": "session-a", "role": "user", "content": "hello",
            "metadata": {"source": "router-test"}, "operation_key": "router-operation"
        });
        if let Some(owner) = owner {
            args["user_identifier"] = owner.into();
        }
        let response = request(state, "add_message", &args.to_string()).await;
        assert_eq!(response.status(), StatusCode::OK);
        let calls = calls.lock().unwrap();
        assert_eq!(
            calls.ensured,
            vec![("session-a".into(), None, owner.map(str::to_owned))]
        );
        assert_eq!(calls.appended.len(), 1);
        let (id, _, message) = &calls.appended[0];
        assert_eq!(*id, conversation_id);
        assert_eq!(message.content, "hello");
        assert_eq!(message.role, "user");
        assert_eq!(message.metadata["source"], "router-test");
        assert_eq!(message.operation_key.as_deref(), Some("router-operation"));
        let payload: serde_json::Value =
            serde_json::from_str(message.operation_payload.as_deref().unwrap()).unwrap();
        assert_eq!(payload["content"], "hello");
        assert_eq!(payload["generate_embedding"], true);
        drop(calls);
    }
}

#[tokio::test]
async fn add_message_rejects_blank_owner_before_dependencies() {
    for multi_tenant in [false, true] {
        for owner in ["", " ", "\t\n"] {
            let calls = Arc::new(Mutex::new(AddCalls::default()));
            let memory = Arc::new(MemoryClient::new(
                MemoryPorts {
                    short_term: Arc::new(UnreachableStore::Add {
                        calls: calls.clone(),
                        conversation_id: Uuid::new_v4(),
                    }),
                    long_term: Arc::new(UnreachableLongTerm),
                    reasoning: Arc::new(UnreachableReasoning),
                    embedder: None,
                    cypher_query: None,
                },
                &MemoryOptions { multi_tenant },
            ));
            let state = Arc::new(BridgeState {
                memory,
                maintenance: None,
            });
            let body = serde_json::json!({
                "session_id": "shared-session",
                "role": "user",
                "content": "must not be written",
                "user_identifier": owner
            });
            let status = post(state, "add_message", &body.to_string()).await;
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "blank scope must fail closed"
            );
            let calls = calls.lock().unwrap();
            assert!(calls.ensured.is_empty());
            assert!(calls.appended.is_empty());
            drop(calls);
        }
    }
}

#[tokio::test]
async fn operation_key_replay_does_not_call_embedding_provider_again() {
    let calls = Arc::new(Mutex::new(AddCalls::default()));
    let embedding_calls = Arc::new(AtomicUsize::new(0));
    let memory = Arc::new(MemoryClient::new(
        MemoryPorts {
            short_term: Arc::new(UnreachableStore::Add {
                calls: calls.clone(),
                conversation_id: Uuid::new_v4(),
            }),
            long_term: Arc::new(UnreachableLongTerm),
            reasoning: Arc::new(UnreachableReasoning),
            embedder: Some(Arc::new(FailOnSecondEmbedder(embedding_calls.clone()))),
            cypher_query: None,
        },
        &MemoryOptions::default(),
    ));
    let state = Arc::new(BridgeState {
        memory,
        maintenance: None,
    });
    let payload = serde_json::json!({
        "session_id": "replay-session",
        "role": "user",
        "content": "persist once",
        "operation_key": "replay-operation"
    })
    .to_string();

    let first = request(state.clone(), "add_message", &payload).await;
    assert_eq!(first.status(), StatusCode::OK);
    let first_body = axum::body::to_bytes(first.into_body(), usize::MAX)
        .await
        .unwrap();
    let first_message: serde_json::Value = serde_json::from_slice(&first_body).unwrap();

    let replay = request(state.clone(), "add_message", &payload).await;
    assert_eq!(replay.status(), StatusCode::OK);
    let replay_body = axum::body::to_bytes(replay.into_body(), usize::MAX)
        .await
        .unwrap();
    let replay_message: serde_json::Value = serde_json::from_slice(&replay_body).unwrap();
    assert_eq!(replay_message["id"], first_message["id"]);

    let conflict_payload = serde_json::json!({
        "session_id": "replay-session",
        "role": "user",
        "content": "different payload",
        "operation_key": "replay-operation"
    })
    .to_string();
    assert_eq!(
        post(state, "add_message", &conflict_payload).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(embedding_calls.load(Ordering::SeqCst), 1);
    assert_eq!(calls.lock().unwrap().appended.len(), 1);
    assert_eq!(
        calls.lock().unwrap().appended[0].2.embedding_identity,
        Some(EmbeddingIdentity::new("bridge:test:fixed-vector:v1", 1).unwrap())
    );
}

#[tokio::test]
async fn add_message_rejects_invalid_owner_types_before_store() {
    for owner in [
        serde_json::json!(42),
        serde_json::json!(false),
        serde_json::json!([]),
        serde_json::json!({}),
    ] {
        let args = serde_json::json!({
            "session_id": "session-a", "role": "user", "content": "hello",
            "user_identifier": owner
        });
        let response = request(state(None, None), "add_message", &args.to_string()).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(String::from_utf8(body.to_vec())
            .unwrap()
            .contains("user_identifier"));
    }
}

#[tokio::test]
async fn multi_tenant_search_requires_owner_before_dependencies() {
    let memory = MemoryClient::new(
        MemoryPorts {
            short_term: Arc::new(UnreachableStore::Never),
            long_term: Arc::new(UnreachableLongTerm),
            reasoning: Arc::new(UnreachableReasoning),
            embedder: Some(Arc::new(UnreachableEmbedder)),
            cypher_query: None,
        },
        &MemoryOptions { multi_tenant: true },
    );
    let state = Arc::new(BridgeState {
        memory: Arc::new(memory),
        maintenance: None,
    });
    for owner in [None, Some(""), Some("   \t\n")] {
        let mut args = serde_json::json!({ "query": "private messages" });
        if let Some(owner) = owner {
            args["user_identifier"] = serde_json::json!(owner);
        }
        let response = request(state.clone(), "search_messages", &args.to_string()).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains("user_identifier"), "{body}");
    }
}

#[tokio::test]
async fn search_passes_optional_owner_and_session_to_store() {
    for multi_tenant in [false, true] {
        for owner in [None, Some("owner-a")] {
            if multi_tenant && owner.is_none() {
                continue;
            }
            for session in [None, Some("shared-session")] {
                let state = Arc::new(BridgeState {
                    memory: Arc::new(MemoryClient::new(
                        MemoryPorts {
                            short_term: Arc::new(UnreachableStore::Search {
                                owner: owner.map(str::to_owned),
                                session: session.map(str::to_owned),
                            }),
                            long_term: Arc::new(UnreachableLongTerm),
                            reasoning: Arc::new(UnreachableReasoning),
                            embedder: Some(Arc::new(SearchEmbedder)),
                            cypher_query: None,
                        },
                        &MemoryOptions { multi_tenant },
                    )),
                    maintenance: None,
                });
                let args = serde_json::json!({"query":"hello", "user_identifier":owner, "session_id":session});
                assert_eq!(
                    post(state, "search_messages", &args.to_string()).await,
                    StatusCode::OK
                );
            }
        }
    }
}

#[tokio::test]
async fn empty_batch_still_requires_valid_owner() {
    let memory = MemoryClient::new(
        MemoryPorts {
            short_term: Arc::new(UnreachableStore::Never),
            long_term: Arc::new(UnreachableLongTerm),
            reasoning: Arc::new(UnreachableReasoning),
            embedder: None,
            cypher_query: None,
        },
        &MemoryOptions { multi_tenant: true },
    );
    for owner in [None, Some(""), Some(" ")] {
        assert!(memory
            .short_term()
            .add_messages_batch("s", &[], 10, false, owner)
            .await
            .is_err());
    }
}

#[tokio::test]
async fn list_conversations_preserves_owner_scope() {
    for multi_tenant in [false, true] {
        let memory = Arc::new(MemoryClient::new(
            MemoryPorts {
                short_term: Arc::new(UnreachableStore::List {
                    owner: OwnerScope::owned(" owner ").unwrap(),
                }),
                long_term: Arc::new(UnreachableLongTerm),
                reasoning: Arc::new(UnreachableReasoning),
                embedder: None,
                cypher_query: None,
            },
            &MemoryOptions { multi_tenant },
        ));
        let state = Arc::new(BridgeState {
            memory,
            maintenance: None,
        });
        assert_eq!(
            post(
                state,
                "list_conversations",
                r#"{"user_identifier":" owner "}"#
            )
            .await,
            StatusCode::OK
        );
    }
}

#[tokio::test]
async fn multi_tenant_short_term_routes_reject_invalid_scope_before_store() {
    let id = Uuid::new_v4().to_string();
    let cases = [
        ("get_conversation", serde_json::json!({"session_id":"s"})),
        (
            "get_conversation_metadata",
            serde_json::json!({"conversation_id": id}),
        ),
        ("list_conversations", serde_json::json!({})),
        ("list_sessions", serde_json::json!({})),
        ("delete_message", serde_json::json!({"message_id":id})),
        (
            "delete_conversation",
            serde_json::json!({"conversation_id":id}),
        ),
        ("clear_session", serde_json::json!({"session_id":"s"})),
        ("create_conversation", serde_json::json!({"session_id":"s"})),
        (
            "bulk_add_messages",
            serde_json::json!({"conversation_id":id,"messages":[]}),
        ),
    ];
    for (method, body) in cases {
        for owner in [
            None,
            Some(serde_json::json!("")),
            Some(serde_json::json!(" \t")),
            Some(serde_json::json!(42)),
        ] {
            let memory = Arc::new(MemoryClient::new(
                MemoryPorts {
                    short_term: Arc::new(UnreachableStore::Never),
                    long_term: Arc::new(UnreachableLongTerm),
                    reasoning: Arc::new(UnreachableReasoning),
                    embedder: None,
                    cypher_query: None,
                },
                &MemoryOptions { multi_tenant: true },
            ));
            let mut args = body.clone();
            if let Some(owner) = owner {
                args["user_identifier"] = owner;
            }
            let state = Arc::new(BridgeState {
                memory,
                maintenance: None,
            });
            assert_eq!(
                post(state, method, &args.to_string()).await,
                StatusCode::BAD_REQUEST,
                "{method}"
            );
        }
    }
}

#[tokio::test]
async fn bulk_add_rejects_malformed_payloads_before_store_access() {
    let id = Uuid::new_v4().to_string();
    let too_many = serde_json::Value::Array(
        (0..101)
            .map(|_| serde_json::json!({"role":"user","content":"x"}))
            .collect(),
    );
    let cases = [
        serde_json::json!({"conversation_id":id}),
        serde_json::json!({"conversation_id":id,"messages":"not-a-list"}),
        serde_json::json!({"conversation_id":id,"messages":too_many}),
        serde_json::json!({"conversation_id":id,"messages":[null]}),
        serde_json::json!({"conversation_id":id,"messages":[{}]}),
        serde_json::json!({"conversation_id":id,"messages":[{"role":"alien","content":"x"}]}),
        serde_json::json!({"conversation_id":id,"messages":[{"role":"user"}]}),
        serde_json::json!({"conversation_id":id,"messages":[{"role":"user","content":"x","metadata":[]}]}),
    ];
    for args in cases {
        assert_eq!(
            post(state(None, None), "bulk_add_messages", &args.to_string()).await,
            StatusCode::BAD_REQUEST,
            "{args}"
        );
    }
}

#[tokio::test]
async fn delete_message_with_unparseable_id_returns_false_without_store_access() {
    let response = request(
        state(None, None),
        "delete_message",
        r#"{"message_id":"not-a-uuid"}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value, serde_json::json!({"deleted":false}));
}

#[tokio::test]
async fn invalid_bridge_arguments_fail_before_short_term_store_access() {
    let cases = [
        (
            "add_message",
            serde_json::json!({"session_id":"s","role":"alien","content":"x"}),
        ),
        ("get_conversation", serde_json::json!({})),
        (
            "search_messages",
            serde_json::json!({"query":"x","metadata_filters":[]}),
        ),
        ("create_conversation", serde_json::json!({"session_id":"s"})),
    ];
    for (method, args) in cases {
        assert_eq!(
            post(state(None, None), method, &args.to_string()).await,
            StatusCode::BAD_REQUEST,
            "{method}: {args}"
        );
    }
}

#[tokio::test]
async fn short_term_context_requires_scope_even_without_dependencies() {
    let memory = MemoryClient::new(
        MemoryPorts {
            short_term: Arc::new(UnreachableStore::Never),
            long_term: Arc::new(UnreachableLongTerm),
            reasoning: Arc::new(UnreachableReasoning),
            embedder: None,
            cypher_query: None,
        },
        &MemoryOptions { multi_tenant: true },
    );
    assert!(memory.short_term().get_context("q", None, 5).await.is_err());
}

#[tokio::test]
async fn short_term_context_preserves_exact_owner_in_search() {
    let memory = MemoryClient::new(
        MemoryPorts {
            short_term: Arc::new(UnreachableStore::Search {
                owner: Some(" owner ".into()),
                session: None,
            }),
            long_term: Arc::new(UnreachableLongTerm),
            reasoning: Arc::new(UnreachableReasoning),
            embedder: Some(Arc::new(SearchEmbedder)),
            cypher_query: None,
        },
        &MemoryOptions { multi_tenant: true },
    );
    assert!(memory
        .short_term()
        .get_context_scoped("q", None, 5, Some(" owner "))
        .await
        .is_ok());
}

struct IngestExtractor {
    calls: AtomicUsize,
    fail: bool,
}
#[async_trait]
impl muninn_core::extraction::EntityExtractor for IngestExtractor {
    async fn extract(
        &self,
        text: &str,
        options: &muninn_core::extraction::ExtractOptions,
    ) -> MemoryResult<muninn_core::extraction::ExtractionResult> {
        use muninn_core::extraction::{ExtractedEntity, ExtractionResult};
        assert_eq!(text, "Alice in Paris");
        assert_eq!(options.extract_relations, Some(false));
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(MemoryError::provider("synthetic extraction unavailable"));
        }
        Ok(ExtractionResult {
            entities: [0.849999, 0.85]
                .into_iter()
                .map(|confidence| ExtractedEntity {
                    name: "Alice".into(),
                    entity_type: "PERSON".into(),
                    subtype: None,
                    start_pos: Some(0),
                    end_pos: Some(5),
                    confidence,
                    extractor: Some("spacy".into()),
                })
                .collect(),
            ..Default::default()
        })
    }
}

fn ingest_state(
    calls: Arc<Mutex<AddCalls>>,
    store: Arc<RecordingLongTerm>,
    extractor: Arc<dyn muninn_core::extraction::EntityExtractor>,
    embedder: Option<Arc<dyn muninn_core::ports::Embedder>>,
) -> Arc<BridgeState> {
    let memory = MemoryClient::new(
        MemoryPorts {
            short_term: Arc::new(UnreachableStore::Add {
                calls,
                conversation_id: Uuid::from_u128(42),
            }),
            long_term: store,
            reasoning: Arc::new(UnreachableReasoning),
            embedder,
            cypher_query: None,
        },
        &MemoryOptions::default(),
    )
    .with_extractor(extractor);
    Arc::new(BridgeState {
        memory: Arc::new(memory),
        maintenance: None,
    })
}

async fn ingest_post(
    state: Arc<BridgeState>,
    extra: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let mut args = serde_json::json!({"session_id":"ingest", "role":"user", "content":"Alice in Paris", "generate_embedding":false, "extract_relations":false});
    args.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    let response = request(state, "add_message", &args.to_string()).await;
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn extraction_auto_populates_provenance_and_replay_is_idempotent() {
    let calls = Arc::new(Mutex::new(AddCalls::default()));
    let store = Arc::new(RecordingLongTerm::default());
    let extractor = Arc::new(IngestExtractor {
        calls: AtomicUsize::new(0),
        fail: false,
    });
    let state = ingest_state(calls.clone(), store.clone(), extractor.clone(), None);
    let args = serde_json::json!({"extraction_mode":"auto", "operation_key":"once"});
    let (status, message) = ingest_post(state.clone(), args.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(store.entities.lock().unwrap().len(), 1);
    let mentions = store.mentions.lock().unwrap().clone();
    assert_eq!(mentions.len(), 1);
    assert_eq!(mentions[0].0.to_string(), message["id"]);
    assert_eq!(mentions[0].1, store.entities.lock().unwrap()[0].id);
    assert_eq!(
        mentions[0].2,
        serde_json::json!({"confidence":0.85,"start_pos":0,"end_pos":5,
        "derived":true,"extractor":"spacy","support":1,"source_message_ids":[message["id"]],
        "evidence":"Alice in Paris"})
    );
    let (_, replay) = ingest_post(state.clone(), args).await;
    assert_eq!(message, replay);
    assert_eq!(calls.lock().unwrap().appended.len(), 1);
    assert_eq!(extractor.calls.load(Ordering::SeqCst), 1);
    assert_eq!(store.mentions.lock().unwrap().len(), 1);
    let (status, _) = ingest_post(
        state,
        serde_json::json!({"extraction_mode":"skip","operation_key":"once"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(calls.lock().unwrap().appended.len(), 1);
    assert_eq!(store.mentions.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn extraction_explicit_only_links_supplied_references_with_existing_precedence() {
    let calls = Arc::new(Mutex::new(AddCalls::default()));
    let store = Arc::new(RecordingLongTerm::default());
    let existing = target_entity(Uuid::new_v4());
    store.entities.lock().unwrap().push(existing.clone());
    let extractor = Arc::new(IngestExtractor {
        calls: AtomicUsize::new(0),
        fail: true,
    });
    let state = ingest_state(calls.clone(), store.clone(), extractor.clone(), None);
    let (status, _) = ingest_post(
        state,
        serde_json::json!({"extraction_mode":"explicit", "explicit_mentions":[
        {"id":existing.id,"name":"ignored","type":"EVENT"}, {"name":"Typed","type":"PERSON"},
        {"name":"unknown"}, {"id":Uuid::new_v4()}]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(extractor.calls.load(Ordering::SeqCst), 0);
    assert_eq!(store.entities.lock().unwrap().len(), 2);
    let mentions = store.mentions.lock().unwrap();
    assert_eq!(mentions.len(), 2);
    assert_eq!(mentions[0].1, existing.id);
    assert!(mentions.iter().all(|(_, _, data)| data["derived"] == false));
    drop(mentions);
    assert_eq!(calls.lock().unwrap().appended.len(), 1);
}

#[tokio::test]
async fn extraction_skip_disabled_and_unconfigured_preserve_no_extraction() {
    // The pinned default extracts on every message once a stage is configured,
    // so the unconfigured case now expects one call; skip and
    // extract_entities=false still suppress it entirely.
    for (extra, expected_calls) in [
        (
            serde_json::json!({"extraction_mode":"skip","explicit_mentions":[{"name":"Typed","type":"PERSON"}]}),
            0,
        ),
        (
            serde_json::json!({"extraction_mode":"auto","extract_entities":false}),
            0,
        ),
        (serde_json::json!({}), 1),
    ] {
        let calls = Arc::new(Mutex::new(AddCalls::default()));
        let store = Arc::new(RecordingLongTerm::default());
        // The unconfigured case runs the pinned auto default, so the stage
        // must work; skip and extract_entities=false never reach it.
        let extractor = Arc::new(IngestExtractor {
            calls: AtomicUsize::new(0),
            fail: false,
        });
        let (status, _) = ingest_post(
            ingest_state(calls.clone(), store.clone(), extractor.clone(), None),
            extra,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(extractor.calls.load(Ordering::SeqCst), expected_calls);
        if expected_calls == 0 {
            assert!(store.entities.lock().unwrap().is_empty());
            assert!(store.mentions.lock().unwrap().is_empty());
        } else {
            // The working stage extracted and persisted the pinned 0.85 span;
            // its 0.849999 sibling stays below the pinned confidence filter.
            assert_eq!(store.entities.lock().unwrap().len(), 1);
            assert_eq!(store.mentions.lock().unwrap().len(), 1);
        }
        assert_eq!(calls.lock().unwrap().appended.len(), 1);
    }
}

#[tokio::test]
async fn extraction_failure_and_invalid_controls_leave_all_state_unchanged() {
    for (extra, status, error) in [
        (
            serde_json::json!({"extraction_mode":"auto"}),
            StatusCode::INTERNAL_SERVER_ERROR,
            "provider error: synthetic extraction unavailable",
        ),
        (
            serde_json::json!({"extraction_mode":"bogus"}),
            StatusCode::BAD_REQUEST,
            "'extraction_mode' must be auto, skip or explicit",
        ),
        (
            serde_json::json!({"extract_entities":"true"}),
            StatusCode::BAD_REQUEST,
            "'extract_entities' must be a boolean",
        ),
        (
            serde_json::json!({"extract_relations":"false"}),
            StatusCode::BAD_REQUEST,
            "'extract_relations' must be a boolean",
        ),
        (
            serde_json::json!({"extraction_mode":"explicit","explicit_mentions":[{}]}),
            StatusCode::BAD_REQUEST,
            "missing argument 'name'",
        ),
    ] {
        let calls = Arc::new(Mutex::new(AddCalls::default()));
        let store = Arc::new(RecordingLongTerm::default());
        let extractor = Arc::new(IngestExtractor {
            calls: AtomicUsize::new(0),
            fail: true,
        });
        let (actual, body) = ingest_post(
            ingest_state(calls.clone(), store.clone(), extractor, None),
            extra,
        )
        .await;
        assert_eq!(actual, status);
        assert_eq!(body, serde_json::json!({"error":error}));
        let calls = calls.lock().unwrap();
        assert!(calls.ensured.is_empty() && calls.appended.is_empty());
        drop(calls);
        assert!(store.entities.lock().unwrap().is_empty());
        assert!(store.mentions.lock().unwrap().is_empty());
        assert!(store.same_as.lock().unwrap().is_empty());
        assert!(store.candidate_requests.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn extraction_embedding_failure_precedes_conversations_nodes_edges_and_vectors() {
    let calls = Arc::new(Mutex::new(AddCalls::default()));
    let store = Arc::new(RecordingLongTerm::default());
    let extractor = Arc::new(IngestExtractor {
        calls: AtomicUsize::new(0),
        fail: false,
    });
    let embedding_calls = Arc::new(AtomicUsize::new(0));
    let state = ingest_state(
        calls.clone(),
        store.clone(),
        extractor,
        Some(Arc::new(FailOnSecondEmbedder(embedding_calls.clone()))),
    );
    let (status, body) = ingest_post(
        state,
        serde_json::json!({"extraction_mode":"auto","generate_embedding":true}),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body,
        serde_json::json!({"error":"provider error: embedding provider unavailable"})
    );
    assert_eq!(embedding_calls.load(Ordering::SeqCst), 2);
    assert!(calls.lock().unwrap().ensured.is_empty());
    assert!(calls.lock().unwrap().appended.is_empty());
    assert!(store.entities.lock().unwrap().is_empty());
    assert!(store.mentions.lock().unwrap().is_empty());
    assert!(store.same_as.lock().unwrap().is_empty());
    assert!(store.candidate_requests.lock().unwrap().is_empty());
}

struct FrozenExtraction(muninn_core::extraction::ExtractionResult);
#[async_trait]
impl muninn_core::extraction::EntityExtractor for FrozenExtraction {
    async fn extract(
        &self,
        _: &str,
        _: &muninn_core::extraction::ExtractOptions,
    ) -> MemoryResult<muninn_core::extraction::ExtractionResult> {
        Ok(self.0.clone())
    }
}

fn extracted_alice() -> muninn_core::extraction::ExtractedEntity {
    muninn_core::extraction::ExtractedEntity {
        name: "Alice".into(),
        entity_type: "PERSON".into(),
        subtype: None,
        start_pos: Some(0),
        end_pos: Some(5),
        confidence: 0.85,
        extractor: Some("spacy".into()),
    }
}

async fn assert_rejected_extraction(entity: muninn_core::extraction::ExtractedEntity) {
    let calls = Arc::new(Mutex::new(AddCalls::default()));
    let store = Arc::new(RecordingLongTerm::default());
    let state = ingest_state(
        calls.clone(),
        store.clone(),
        Arc::new(FrozenExtraction(
            muninn_core::extraction::ExtractionResult {
                entities: vec![entity],
                ..Default::default()
            },
        )),
        None,
    );
    let (status, body) = ingest_post(state, serde_json::json!({"extraction_mode":"auto"})).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body,
        serde_json::json!({"error":"provider error: extraction invalid entity"})
    );
    assert!(calls.lock().unwrap().ensured.is_empty());
    assert!(calls.lock().unwrap().appended.is_empty());
    assert!(store.entities.lock().unwrap().is_empty());
    assert!(store.mentions.lock().unwrap().is_empty());
    assert!(store.same_as.lock().unwrap().is_empty());
    assert!(store.candidate_requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn extraction_invalid_provider_type_is_rejected_before_writes() {
    let mut entity = extracted_alice();
    entity.entity_type = "PERSON) SET malicious=true".into();
    assert_rejected_extraction(entity).await;
}

#[tokio::test]
async fn extraction_invalid_provider_span_is_rejected_before_writes() {
    for (start, end) in [
        (Some(-1), Some(5)),
        (Some(5), Some(0)),
        (Some(0), Some(99)),
        (None, Some(5)),
    ] {
        let mut entity = extracted_alice();
        entity.start_pos = start;
        entity.end_pos = end;
        assert_rejected_extraction(entity).await;
    }
}

#[tokio::test]
async fn extraction_invalid_provider_confidence_is_rejected_before_writes() {
    for confidence in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
        let mut entity = extracted_alice();
        entity.confidence = confidence;
        assert_rejected_extraction(entity).await;
    }
}

#[tokio::test]
async fn extraction_explicit_ambiguity_missing_ids_and_name_only_never_fall_back() {
    for count in [0, 1, 2] {
        let calls = Arc::new(Mutex::new(AddCalls::default()));
        let store = Arc::new(RecordingLongTerm::default());
        for i in 0..count {
            let mut entity = target_entity(Uuid::new_v4());
            entity.name = format!("canonical-{i}");
            entity.canonical_name = Some(entity.name.clone());
            entity.aliases = vec!["Alice".into()];
            store.entities.lock().unwrap().push(entity);
        }
        let before = serde_json::to_value(&*store.entities.lock().unwrap()).unwrap();
        let extractor = Arc::new(IngestExtractor {
            calls: AtomicUsize::new(0),
            fail: true,
        });
        let state = ingest_state(calls.clone(), store.clone(), extractor.clone(), None);
        let (status, _) = ingest_post(
            state,
            serde_json::json!({"extraction_mode":"explicit","explicit_mentions":[
                {"id":Uuid::new_v4(),"name":"Alice","type":"PERSON"},
                {"id":"malformed-id","name":"Alice"}, {"name":" alice "}, {"name":"unknown"}
            ]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(extractor.calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            serde_json::to_value(&*store.entities.lock().unwrap()).unwrap(),
            before
        );
        assert_eq!(
            store.mentions.lock().unwrap().len(),
            usize::from(count == 1)
        );
        assert!(store.same_as.lock().unwrap().is_empty());
        assert_eq!(calls.lock().unwrap().appended.len(), 1);
    }
}

#[tokio::test]
async fn extraction_unconfigured_preserves_operation_payload_metadata_owner_and_vectors() {
    // Without a stage the legacy payload is byte-for-byte preserved; with one,
    // the pinned auto default extracts, so this pins the no-stage half.
    for configured in [false] {
        let calls = Arc::new(Mutex::new(AddCalls::default()));
        let store = Arc::new(RecordingLongTerm::default());
        let stage = Arc::new(IngestExtractor {
            calls: AtomicUsize::new(0),
            fail: true,
        });
        let embedder_calls = Arc::new(AtomicUsize::new(0));
        let ports = MemoryPorts {
            short_term: Arc::new(UnreachableStore::Add {
                calls: calls.clone(),
                conversation_id: Uuid::from_u128(42),
            }),
            long_term: store.clone(),
            reasoning: Arc::new(UnreachableReasoning),
            embedder: Some(Arc::new(FailOnSecondEmbedder(embedder_calls.clone()))),
            cypher_query: None,
        };
        let mut memory = MemoryClient::new(ports, &MemoryOptions::default());
        if configured {
            memory = memory.with_extractor(stage.clone());
        }
        let state = Arc::new(BridgeState {
            memory: Arc::new(memory),
            maintenance: None,
        });
        let args = serde_json::json!({"operation_key":"legacy", "session_id":"ingest",
            "role":"user","content":"Alice in Paris","metadata":{"synthetic":true},"user_identifier":" owner "});
        let mut responses = Vec::new();
        for _ in 0..2 {
            let response = request(state.clone(), "add_message", &args.to_string()).await;
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            responses.push(serde_json::from_slice::<serde_json::Value>(&bytes).unwrap());
        }
        let first = &responses[0];
        assert_eq!(first, &responses[1]);
        assert_eq!(first["embedding"], serde_json::json!([1.0]));
        assert_eq!(first["metadata"], serde_json::json!({"synthetic":true}));
        assert_eq!(stage.calls.load(Ordering::SeqCst), 0);
        assert_eq!(embedder_calls.load(Ordering::SeqCst), 1);
        assert!(store.entities.lock().unwrap().is_empty());
        assert!(store.mentions.lock().unwrap().is_empty());
        let calls = calls.lock().unwrap();
        assert_eq!(calls.appended.len(), 1);
        assert_eq!(calls.ensured.len(), 2);
        assert_eq!(calls.appended[0].1.as_str(), Some(" owner "));
        assert_eq!(calls.appended[0].2.operation_payload,Some(serde_json::json!({
            "role":"user","content":"Alice in Paris","metadata":{"synthetic":true},"generate_embedding":true
        }).to_string()));
    }
}

type SameAsRecord = (Uuid, Uuid, f64, String, String);

/// Long-term store that records what the bridge actually forwards.
#[derive(Default)]
struct RecordingLongTerm {
    fact: Mutex<Option<Fact>>,
    preference: Mutex<Option<Preference>>,
    applies_to: Mutex<Vec<(Uuid, Uuid)>>,
    about: Mutex<Vec<(Uuid, Uuid)>>,
    relationship: Mutex<Option<Relationship>>,
    facts: Mutex<Vec<Fact>>,
    preferences: Mutex<Vec<Preference>>,
    supersessions: Mutex<Vec<(Uuid, Uuid)>>,
    preferences_query: Mutex<Option<(String, bool)>>,
    preference_target: Mutex<Option<Uuid>>,
    preference_as_of: Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    entities: Mutex<Vec<Entity>>,
    same_as: Mutex<Vec<SameAsRecord>>,
    mentions: Mutex<Vec<(Uuid, Uuid, serde_json::Value)>>,
    candidate_score: f64,
    candidate_requests: Mutex<Vec<i64>>,
}

impl RecordingLongTerm {
    fn stored_fact(&self) -> Fact {
        self.fact.lock().unwrap().clone().expect("fact not stored")
    }

    fn set_preferences(&self, preferences: Vec<Preference>) {
        *self.preferences.lock().unwrap() = preferences;
    }

    fn supersessions(&self) -> Vec<(Uuid, Uuid)> {
        self.supersessions.lock().unwrap().clone()
    }

    fn last_preferences_query(&self) -> Option<(String, bool)> {
        self.preferences_query.lock().unwrap().clone()
    }

    fn set_facts(&self, facts: Vec<Fact>) {
        *self.facts.lock().unwrap() = facts;
    }

    fn stored_relationship(&self) -> Relationship {
        self.relationship
            .lock()
            .unwrap()
            .clone()
            .expect("relationship not stored")
    }

    fn applies_to_records(&self) -> Vec<(Uuid, Uuid)> {
        self.applies_to.lock().unwrap().clone()
    }

    fn about_records(&self) -> Vec<(Uuid, Uuid)> {
        self.about.lock().unwrap().clone()
    }

    fn stored_preference(&self) -> Preference {
        self.preference
            .lock()
            .unwrap()
            .clone()
            .expect("preference not stored")
    }
}

// Resolution fixtures are test-only: controlled scores are not semantic-quality evidence.
async fn resolution_post(
    store: Arc<RecordingLongTerm>,
    args: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let response = request(state_with_long_term(store), "add_entity", &args.to_string()).await;
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

fn resolution_store(score: f64) -> Arc<RecordingLongTerm> {
    let store = Arc::new(RecordingLongTerm {
        candidate_score: score,
        ..Default::default()
    });
    let mut candidate = target_entity(Uuid::new_v4());
    candidate.name = "zzzzzzzzzzzzzzzzzzzz".into();
    candidate.canonical_name = Some(candidate.name.clone());
    candidate.entity_type = "PERSON".into();
    store.entities.lock().unwrap().push(candidate);
    store
}

async fn assert_resolution_band(score: f64, action: &str, nodes: usize, edges: usize) {
    let store = resolution_store(score);
    let original = store.entities.lock().unwrap()[0].clone();
    let (status, value) = resolution_post(store.clone(), serde_json::json!({"name":"abcdefghijklmnopqrst", "entity_type":"PERSON", "deduplicate":true})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["deduplication"]["action"], action);
    assert_eq!(value["deduplication"]["is_duplicate"], action != "none");
    assert_eq!(store.entities.lock().unwrap().len(), nodes);
    assert_eq!(store.same_as.lock().unwrap().len(), edges);
    assert_eq!(*store.candidate_requests.lock().unwrap(), vec![12]);
    if action == "merged" {
        assert_eq!(value["id"], original.id.to_string());
        assert_eq!(value["deduplication"]["similarity_score"], score);
        assert_eq!(
            store.entities.lock().unwrap()[0].aliases,
            vec!["abcdefghijklmnopqrst"]
        );
    } else {
        assert_ne!(value["id"], original.id.to_string());
        assert_eq!(
            serde_json::to_value(&store.entities.lock().unwrap()[0]).unwrap(),
            serde_json::to_value(&original).unwrap()
        );
    }
    if action == "flagged" {
        let edge = store.same_as.lock().unwrap()[0].clone();
        assert_eq!(edge.0.to_string(), value["id"].as_str().unwrap());
        assert_eq!(edge.1, original.id);
        assert_eq!(edge.2, score);
        assert_eq!(edge.3, "embedding");
        assert_eq!(edge.4, "pending");
        let (_, repeated) = resolution_post(store.clone(), serde_json::json!({"name":"abcdefghijklmnopqrst", "entity_type":"PERSON", "deduplicate":true})).await;
        assert_eq!(repeated["id"], value["id"]);
        assert_eq!(store.entities.lock().unwrap().len(), 2);
        assert_eq!(store.same_as.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn resolution_at_review_boundary_flags() {
    assert_resolution_band(0.85, "flagged", 2, 1).await;
}
#[tokio::test]
async fn resolution_at_auto_merge_boundary_merges() {
    assert_resolution_band(0.90, "merged", 1, 0).await;
}
#[tokio::test]
async fn resolution_below_review_is_distinct() {
    assert_resolution_band(0.849999, "none", 2, 0).await;
}
#[tokio::test]
async fn resolution_below_auto_merge_stays_pending() {
    assert_resolution_band(0.899999, "flagged", 2, 1).await;
}

#[tokio::test]
async fn resolution_fuzzy_combines_with_the_embedding_score() {
    // Pinned combination: (embedding + fuzz.ratio) / 2 over the name and the
    // canonical name. Without an embedding (score 0.0) even a similar name
    // stays below the flag boundary, because fuzzy never scores alone.
    let store = resolution_store(0.0);
    let (status, value) = resolution_post(store.clone(), serde_json::json!({"name":"qqqqqqqqqqqqqqqqqqqq","entity_type":"PERSON","resolve":true,"generate_embedding":false})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["deduplication"]["action"], "none");
    assert_eq!(store.entities.lock().unwrap().len(), 2);
    assert!(store.same_as.lock().unwrap().is_empty());
    // With an embedding at 0.87 (just above the flag boundary), a perfect
    // fuzzy score combines to (0.87 + 1.0) / 2: merged with match_type both.
    let store = resolution_store(0.87);
    let (status, value) = resolution_post(store.clone(), serde_json::json!({"name":"zzzzzzzzzzzzzzzzzzzz","entity_type":"PERSON","resolve":true,"generate_embedding":false})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["deduplication"]["action"], "merged");
    assert_eq!(value["deduplication"]["match_type"], "both");
    let combined = (0.87_f64 + 1.0) / 2.0;
    assert!((value["deduplication"]["similarity_score"].as_f64().unwrap() - combined).abs() < 1e-9);
    assert_eq!(store.entities.lock().unwrap().len(), 1);
    assert!(store.same_as.lock().unwrap().is_empty());
}

#[tokio::test]
async fn resolution_invalid_flags_reject_before_dependencies() {
    for key in ["resolve", "deduplicate", "generate_embedding"] {
        let store = resolution_store(1.0);
        let mut args = serde_json::json!({"name":"incoming","entity_type":"PERSON"});
        args[key] = "false".into();
        let (status, value) = resolution_post(store.clone(), args).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(value["error"], format!("'{key}' must be a boolean"));
        assert_eq!(store.entities.lock().unwrap().len(), 1);
        assert!(store.same_as.lock().unwrap().is_empty());
        assert!(store.candidate_requests.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn resolution_configuration_rejects_before_nodes_edges_or_candidate_calls() {
    for (config, message) in [
        (serde_json::json!({"review_threshold":0.91}), "resolution.review_threshold must be <= resolution.auto_merge_threshold (got 0.91 > 0.9)"),
        (serde_json::json!({"candidate_limit":0}), "candidate_limit must be at least 1"),
        (serde_json::json!({"fuzzy_threshold":1.1}), "fuzzy_threshold must be between 0 and 1"),
        (serde_json::json!({"auto_merge_threshold":-0.1}), "auto_merge_threshold must be between 0 and 1"),
        (serde_json::json!({"semantic_threshold":1.1}), "semantic_threshold must be between 0 and 1"),
        (serde_json::json!({"exact_threshold":1.1}), "exact_threshold must be between 0 and 1"),
    ] {
        let store = resolution_store(1.0);
        let before = serde_json::to_value(&*store.entities.lock().unwrap()).unwrap();
        let (status, value) = resolution_post(store.clone(), serde_json::json!({"name":"incoming", "entity_type":"PERSON", "deduplicate":true, "resolution":config})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(value["error"], format!("invalid input: {message}"));
        assert_eq!(serde_json::to_value(&*store.entities.lock().unwrap()).unwrap(), before);
        assert!(store.same_as.lock().unwrap().is_empty());
        assert!(store.candidate_requests.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn resolution_same_type_and_candidate_budget_are_enforced() {
    let store = resolution_store(0.90);
    store.entities.lock().unwrap()[0].entity_type = "OBJECT".into();
    let (_, value) = resolution_post(store.clone(), serde_json::json!({"name":"incoming", "entity_type":"PERSON", "resolve":true, "resolution":{"candidate_limit":1}})).await;
    assert_eq!(value["deduplication"]["action"], "none");
    assert_eq!(*store.candidate_requests.lock().unwrap(), vec![1]);
    assert_eq!(store.entities.lock().unwrap().len(), 2);
    assert!(store.same_as.lock().unwrap().is_empty());
}

#[tokio::test]
async fn resolution_candidate_budget_limits_consulted_candidates() {
    // Pinned `find_duplicate`: the vector query returns at most candidate_limit
    // rows, so a budget of one consults only the first candidate.
    for (limit, expected) in [(1, "merged"), (12, "merged")] {
        let store = resolution_store(0.9);
        let mut later = target_entity(Uuid::new_v4());
        later.entity_type = "PERSON".into();
        later.name = "different candidate".into();
        store.entities.lock().unwrap().push(later);
        let (status, value) = resolution_post(store.clone(), serde_json::json!({"name":"incoming","entity_type":"PERSON","deduplicate":true,"resolution":{"candidate_limit":limit}})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["deduplication"]["action"], expected);
        assert_eq!(*store.candidate_requests.lock().unwrap(), vec![limit]);
        assert!(store.same_as.lock().unwrap().is_empty());
        assert_eq!(store.entities.lock().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn resolution_aliases_are_never_scored() {
    // Pinned fuzzy scoring checks only the name and canonical name; an alias
    // matching the incoming name must not create a match (score 0.0 stays
    // below the flag boundary, so the result is the distinct default).
    let store = resolution_store(0.0);
    let mut aliased = target_entity(Uuid::new_v4());
    aliased.entity_type = "PERSON".into();
    aliased.name = "different candidate".into();
    aliased.aliases = vec!["incoming".into()];
    store.entities.lock().unwrap().push(aliased);
    let (status, value) = resolution_post(
        store.clone(),
        serde_json::json!({"name":"incoming","entity_type":"PERSON","deduplicate":true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["deduplication"]["action"], "none");
    assert_eq!(store.entities.lock().unwrap().len(), 3);
    assert!(store.same_as.lock().unwrap().is_empty());
}

#[tokio::test]
async fn resolution_exact_alias_beats_an_equal_vector_score() {
    let store = resolution_store(1.0);
    let mut exact = target_entity(Uuid::new_v4());
    exact.entity_type = "PERSON".into();
    exact.name = "canonical candidate".into();
    exact.aliases = vec!["incoming".into()];
    store.entities.lock().unwrap().push(exact.clone());
    let (status, result) = resolution_post(
        store.clone(),
        serde_json::json!({"name":"incoming","entity_type":"PERSON","resolve":true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // Pinned selection keeps the first strict improvement: both candidates
    // carry the same embedding score, and aliases are never scored.
    assert_eq!(result["deduplication"]["match_type"], "embedding");
    assert_eq!(store.entities.lock().unwrap().len(), 2);
    assert!(store.same_as.lock().unwrap().is_empty());
}

#[tokio::test]
async fn resolution_legacy_adapter_fails_explicitly_instead_of_distinct() {
    let response = request(
        state(None, None),
        "add_entity",
        r#"{"name":"incoming","entity_type":"PERSON","resolve":true}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
        serde_json::json!({"error":"not supported: entity resolution candidates"})
    );
}

#[tokio::test]
async fn resolution_opt_out_preserves_top_level_fields_and_existing_upsert() {
    for controls in [
        serde_json::json!({}),
        serde_json::json!({"resolve":false,"deduplicate":false}),
        serde_json::json!({"resolve":null,"deduplicate":null}),
    ] {
        let store = resolution_store(1.0);
        let mut args = controls;
        args["name"] = "incoming".into();
        args["entity_type"] = "person".into();
        args["description"] = "original contract".into();
        let (status, first) = resolution_post(store.clone(), args.clone()).await;
        let (_, second) = resolution_post(store.clone(), args).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(first["id"], second["id"]);
        assert_eq!(first["name"], "incoming");
        assert_eq!(first["type"], "PERSON");
        assert_eq!(first["description"], "original contract");
        assert_eq!(
            first["deduplication"],
            serde_json::json!({"is_duplicate":false,"action":"none","matched_entity_id":null,"matched_entity_name":null,"similarity_score":0.0,"match_type":null})
        );
        assert_eq!(store.entities.lock().unwrap().len(), 2);
        assert!(store.same_as.lock().unwrap().is_empty());
        assert!(store.candidate_requests.lock().unwrap().is_empty());
    }
}

fn timestamp(text: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&chrono::Utc)
}

#[async_trait]
impl LongTermStore for RecordingLongTerm {
    async fn resolution_candidates(
        &self,
        query: &muninn_core::resolution::ResolutionQuery<'_>,
    ) -> MemoryResult<Vec<Scored<Entity>>> {
        self.candidate_requests.lock().unwrap().push(query.limit);
        let mut candidates = self.entities.lock().unwrap().clone();
        candidates.sort_by_key(|entity| entity.name != query.name);
        Ok(candidates
            .into_iter()
            .filter(|e| query.entity_type.is_none_or(|kind| kind == e.entity_type))
            .take(usize::try_from(query.limit).unwrap())
            .map(|item| Scored {
                item,
                score: self.candidate_score,
            })
            .collect())
    }
    async fn upsert_entity(&self, incoming: &Entity) -> MemoryResult<Entity> {
        let mut entities = self.entities.lock().unwrap();
        if let Some(stored) = entities
            .iter_mut()
            .find(|e| e.name == incoming.name && e.entity_type == incoming.entity_type)
        {
            stored.description.clone_from(&incoming.description);
            for alias in &incoming.aliases {
                if !stored.aliases.contains(alias) {
                    stored.aliases.push(alias.clone());
                }
            }
            return Ok(stored.clone());
        }
        entities.push(incoming.clone());
        drop(entities);
        Ok(incoming.clone())
    }
    async fn get_entity(&self, id: Uuid) -> MemoryResult<Option<Entity>> {
        Ok(self
            .entities
            .lock()
            .unwrap()
            .iter()
            .find(|entity| entity.id == id)
            .cloned())
    }
    async fn get_entity_by_name(&self, _: &str) -> MemoryResult<Option<Entity>> {
        unreachable!()
    }
    async fn search_entities(
        &self,
        _: &[f64],
        _: i64,
        _: f64,
    ) -> MemoryResult<Vec<Scored<Entity>>> {
        unreachable!()
    }
    async fn list_entities(&self, kind: Option<&str>, limit: i64) -> MemoryResult<Vec<Entity>> {
        Ok(self
            .entities
            .lock()
            .unwrap()
            .iter()
            .filter(|e| kind.is_none_or(|k| k == e.entity_type))
            .take(usize::try_from(limit).unwrap())
            .cloned()
            .collect())
    }
    async fn update_entity(&self, _: Uuid, _: &EntityUpdate) -> MemoryResult<Option<Entity>> {
        unreachable!()
    }
    async fn delete_entity(&self, _: Uuid) -> MemoryResult<bool> {
        unreachable!()
    }
    async fn related_entities(&self, _: Uuid, _: u8) -> MemoryResult<Vec<RelatedEntity>> {
        Ok(vec![])
    }
    async fn upsert_relationship(
        &self,
        relationship: &Relationship,
    ) -> MemoryResult<Option<Relationship>> {
        *self.relationship.lock().unwrap() = Some(relationship.clone());
        Ok(Some(relationship.clone()))
    }
    async fn merge_entities(&self, _: Uuid, _: Uuid) -> MemoryResult<Option<(Entity, Entity)>> {
        unreachable!()
    }
    async fn link_same_as(
        &self,
        source: Uuid,
        target: Uuid,
        score: f64,
        kind: &str,
        status: &str,
    ) -> MemoryResult<()> {
        let mut edges = self.same_as.lock().unwrap();
        if !edges
            .iter()
            .any(|e| (e.0 == source && e.1 == target) || (e.1 == source && e.0 == target))
        {
            edges.push((source, target, score, kind.into(), status.into()));
        }
        drop(edges);
        Ok(())
    }
    async fn link_message_to_entity_with_provenance(
        &self,
        message: Uuid,
        entity: Uuid,
        provenance: &muninn_core::extraction::MentionProvenance,
    ) -> MemoryResult<bool> {
        let mut mentions = self.mentions.lock().unwrap();
        if !mentions
            .iter()
            .any(|(m, e, _)| *m == message && *e == entity)
        {
            mentions.push((message, entity, serde_json::to_value(provenance).unwrap()));
        }
        drop(mentions);
        Ok(true)
    }
    async fn set_entity_feedback(&self, _: Uuid, _: f64, _: bool) -> MemoryResult<bool> {
        unreachable!()
    }
    async fn link_message_to_entity(&self, _: Uuid, _: Uuid, _: f64) -> MemoryResult<bool> {
        unreachable!()
    }
    async fn entity_mentions(&self, _: Uuid) -> MemoryResult<Option<Vec<EntityMention>>> {
        unreachable!()
    }
    async fn entity_graph(&self, _: i64) -> MemoryResult<EntityGraph> {
        unreachable!()
    }
    async fn create_preference(&self, preference: &Preference) -> MemoryResult<Preference> {
        *self.preference.lock().unwrap() = Some(preference.clone());
        Ok(preference.clone())
    }
    async fn find_duplicate_preference(
        &self,
        _: &[f64],
        _: &str,
        _: f64,
        _: Option<&str>,
    ) -> MemoryResult<Option<Preference>> {
        unreachable!()
    }
    async fn link_user_to_preference(&self, _: &str, _: Uuid) -> MemoryResult<()> {
        unreachable!()
    }
    async fn link_preference_to_entity(
        &self,
        preference_id: Uuid,
        entity_id: Uuid,
    ) -> MemoryResult<()> {
        self.applies_to
            .lock()
            .unwrap()
            .push((preference_id, entity_id));
        Ok(())
    }

    // The ABOUT writer records one entry per distinct pair, mirroring the
    // pinned property-free `MERGE`.
    async fn link_preference_about_entity(
        &self,
        preference_id: Uuid,
        entity_id: Uuid,
    ) -> MemoryResult<()> {
        let mut edges = self.about.lock().unwrap();
        if !edges.contains(&(preference_id, entity_id)) {
            edges.push((preference_id, entity_id));
        }
        drop(edges);
        Ok(())
    }
    async fn supersede_preference(&self, old_id: Uuid, new_id: Uuid) -> MemoryResult<()> {
        self.supersessions.lock().unwrap().push((old_id, new_id));
        Ok(())
    }
    async fn preferences_for(
        &self,
        user_identifier: &str,
        active_only: bool,
    ) -> MemoryResult<Vec<Preference>> {
        *self.preferences_query.lock().unwrap() = Some((user_identifier.to_owned(), active_only));
        Ok(self.preferences.lock().unwrap().clone())
    }
    async fn preferences_for_filtered(
        &self,
        user: &str,
        active_only: bool,
        target: Option<Uuid>,
        as_of: Option<chrono::DateTime<chrono::Utc>>,
    ) -> MemoryResult<Vec<Preference>> {
        *self.preference_target.lock().unwrap() = target;
        *self.preference_as_of.lock().unwrap() = as_of;
        self.preferences_for(user, active_only).await
    }
    async fn search_preferences(
        &self,
        _: &[f64],
        _: i64,
        _: f64,
        _: Option<&str>,
    ) -> MemoryResult<Vec<Scored<Preference>>> {
        unreachable!()
    }
    async fn preferences_by_category(
        &self,
        _: &str,
        _: i64,
        _: Option<&str>,
    ) -> MemoryResult<Vec<Preference>> {
        unreachable!()
    }
    async fn create_fact(&self, fact: &Fact) -> MemoryResult<Fact> {
        *self.fact.lock().unwrap() = Some(fact.clone());
        Ok(fact.clone())
    }
    async fn find_duplicate_fact(
        &self,
        _: &[f64],
        _: &str,
        _: &str,
        _: f64,
    ) -> MemoryResult<Option<Fact>> {
        Ok(None)
    }
    async fn update_fact_confidence(&self, _: Uuid, _: f64, _: &str) -> MemoryResult<()> {
        unreachable!()
    }
    async fn search_facts(&self, _: &[f64], _: i64, _: f64) -> MemoryResult<Vec<Scored<Fact>>> {
        unreachable!()
    }
    async fn facts_by_subject(&self, _: &str, _: i64) -> MemoryResult<Vec<Fact>> {
        Ok(self.facts.lock().unwrap().clone())
    }
}

fn state_with_long_term(long_term: Arc<dyn LongTermStore>) -> Arc<BridgeState> {
    let memory = MemoryClient::new(
        MemoryPorts {
            short_term: Arc::new(UnreachableStore::Never),
            long_term,
            reasoning: Arc::new(UnreachableReasoning),
            embedder: None,
            cypher_query: None,
        },
        &MemoryOptions::default(),
    );
    Arc::new(BridgeState {
        memory: Arc::new(memory),
        maintenance: None,
    })
}

#[tokio::test]
async fn add_fact_forwards_supplied_confidence_and_validity() {
    let store = Arc::new(RecordingLongTerm::default());
    let response = request(
        state_with_long_term(store.clone()),
        "add_fact",
        r#"{"subject":"ana","predicate":"vive_en","obj":"Madrid","confidence":0.9,
            "valid_from":"2026-02-01T00:00:00+00:00",
            "valid_until":"2026-06-01T00:00:00+00:00"}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let fact: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let stored = store.stored_fact();
    assert_eq!(stored.confidence, 0.9);
    assert_eq!(
        stored.valid_from,
        Some(timestamp("2026-02-01T00:00:00+00:00"))
    );
    assert_eq!(
        stored.valid_until,
        Some(timestamp("2026-06-01T00:00:00+00:00"))
    );
    assert_eq!(fact["confidence"], 0.9);
    assert_eq!(fact["valid_from"], "2026-02-01T00:00:00+00:00");
    assert_eq!(fact["valid_until"], "2026-06-01T00:00:00+00:00");
}

#[tokio::test]
async fn add_preference_forwards_confidence_and_metadata() {
    let store = Arc::new(RecordingLongTerm::default());
    let response = request(
        state_with_long_term(store.clone()),
        "add_preference",
        r#"{"category":"food","preference":"cafe sin azucar","context":"manana",
            "confidence":0.8,"metadata":{"source":"gm05","nested":{"tags":["n"]}}}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let stored = store.stored_preference();
    assert_eq!(stored.confidence, 0.8);
    assert_eq!(stored.metadata["source"], "gm05");
    assert_eq!(body["confidence"], 0.8);
    assert_eq!(body["metadata"]["source"], "gm05");
    assert_eq!(body["metadata"]["nested"]["tags"][0], "n");
}

#[tokio::test]
async fn add_preference_links_applies_to_entities() {
    let store = Arc::new(RecordingLongTerm::default());
    let entity = Uuid::new_v4();
    store.entities.lock().unwrap().push(target_entity(entity));
    let body = format!(r#"{{"category":"food","preference":"cafe","applies_to":["{entity}"]}}"#);
    let response = request(state_with_long_term(store.clone()), "add_preference", &body).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        store.applies_to_records(),
        vec![(store.stored_preference().id, entity)]
    );
}

#[tokio::test]
async fn add_preference_without_about_entities_leaves_the_payload_and_graph_unchanged() {
    let store = Arc::new(RecordingLongTerm::default());
    let entity = Uuid::new_v4();
    store.entities.lock().unwrap().push(target_entity(entity));
    let body = format!(r#"{{"category":"food","preference":"cafe","applies_to":["{entity}"]}}"#);
    let response = request(state_with_long_term(store.clone()), "add_preference", &body).await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    // No new wire field is projected, and APPLIES_TO still behaves as before.
    assert_eq!(body["category"], "food");
    assert_eq!(body["preference"], "cafe");
    assert!(body.get("about").is_none());
    assert!(body.get("about_entities").is_none());
    assert_eq!(
        store.applies_to_records(),
        vec![(store.stored_preference().id, entity)]
    );
    assert!(store.about_records().is_empty());
}

#[tokio::test]
async fn add_preference_creates_one_about_edge_only_when_requested() {
    let store = Arc::new(RecordingLongTerm::default());
    let entity = Uuid::new_v4();
    store.entities.lock().unwrap().push(target_entity(entity));
    let body = format!(
        r#"{{"category":"food","preference":"cafe","about_entities":["{entity}","{entity}"]}}"#
    );
    let response = request(state_with_long_term(store.clone()), "add_preference", &body).await;
    assert_eq!(response.status(), StatusCode::OK);
    let preference = store.stored_preference();
    // One property-free edge for the caller's own preference; ABOUT alone does
    // not imply the APPLIES_TO filter edge.
    assert_eq!(store.about_records(), vec![(preference.id, entity)]);
    assert!(store.applies_to_records().is_empty());
}

#[tokio::test]
async fn add_preference_rejects_malformed_about_entities_before_the_store() {
    let store = Arc::new(RecordingLongTerm::default());
    for extra in [
        r#""about_entities":"not-a-list""#,
        r#""about_entities":["x"]"#,
    ] {
        let body = format!(r#"{{"category":"food","preference":"tea",{extra}}}"#);
        let response = request(state_with_long_term(store.clone()), "add_preference", &body).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["error"],
            "'about_entities' must be a list of UUIDs"
        );
        assert!(store.preference.lock().unwrap().is_none());
        assert!(store.applies_to_records().is_empty());
        assert!(store.about_records().is_empty());
    }
}

#[tokio::test]
async fn unknown_about_target_returns_404_and_writes_nothing() {
    let store = Arc::new(RecordingLongTerm::default());
    let target = Uuid::new_v4();
    let missing = Uuid::new_v4();
    store.entities.lock().unwrap().push(target_entity(target));
    let body = serde_json::json!({
        "category": "food",
        "preference": "tea",
        "applies_to": [target.to_string()],
        "about_entities": [target.to_string(), missing.to_string()],
    })
    .to_string();
    let response = request(state_with_long_term(store.clone()), "add_preference", &body).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["error"],
        format!("not found: entity {missing}")
    );
    assert!(store.preference.lock().unwrap().is_none());
    assert!(store.applies_to_records().is_empty());
    assert!(store.about_records().is_empty());
}

#[tokio::test]
async fn add_relationship_forwards_description_confidence_and_validity() {
    let store = Arc::new(RecordingLongTerm::default());
    let source = Uuid::new_v4();
    let target = Uuid::new_v4();
    let body = format!(
        r#"{{"source_id":"{source}","target_id":"{target}",
            "relationship_type":"WORKS_AT","description":"trabaja en",
            "confidence":0.75,"valid_from":"2026-03-01T00:00:00+00:00",
            "valid_until":"2026-09-01T00:00:00+00:00",
            "properties":{{"role":"cto"}}}}"#
    );
    let response = request(
        state_with_long_term(store.clone()),
        "add_relationship",
        &body,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let relationship: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let stored = store.stored_relationship();
    assert_eq!(stored.description.as_deref(), Some("trabaja en"));
    assert_eq!(stored.confidence, 0.75);
    assert_eq!(
        stored.valid_from,
        Some(timestamp("2026-03-01T00:00:00+00:00"))
    );
    assert_eq!(
        stored.valid_until,
        Some(timestamp("2026-09-01T00:00:00+00:00"))
    );
    assert_eq!(stored.properties["role"], "cto");
    assert_eq!(relationship["description"], "trabaja en");
    assert_eq!(relationship["confidence"], 0.75);
    assert_eq!(relationship["valid_from"], "2026-03-01T00:00:00+00:00");
    assert_eq!(relationship["valid_until"], "2026-09-01T00:00:00+00:00");
}

#[tokio::test]
async fn get_facts_about_returns_stored_facts() {
    let store = Arc::new(RecordingLongTerm::default());
    let fact = Fact {
        id: Uuid::new_v4(),
        subject: "ana".into(),
        predicate: "vive_en".into(),
        object: "Madrid".into(),
        confidence: 0.9,
        embedding: None,
        valid_from: Some(timestamp("2026-02-01T00:00:00+00:00")),
        valid_until: None,
        created_at: chrono::Utc::now(),
        metadata: Metadata::new(),
    };
    store.set_facts(vec![fact.clone()]);
    let response = request(
        state_with_long_term(store.clone()),
        "get_facts_about",
        r#"{"subject":"ana","limit":10}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body[0]["id"], fact.id.to_string());
    assert_eq!(body[0]["object"], "Madrid");
    assert_eq!(body[0]["confidence"], 0.9);
    assert_eq!(body[0]["valid_from"], "2026-02-01T00:00:00+00:00");
}

#[tokio::test]
async fn supersede_preference_marks_the_old_preference() {
    let store = Arc::new(RecordingLongTerm::default());
    let old = Uuid::new_v4();
    let new = Uuid::new_v4();
    let body = format!(r#"{{"old_id":"{old}","new_id":"{new}"}}"#);
    let response = request(
        state_with_long_term(store.clone()),
        "supersede_preference",
        &body,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(store.supersessions(), vec![(old, new)]);
}

fn target_entity(id: Uuid) -> Entity {
    Entity {
        id,
        name: "target".into(),
        canonical_name: None,
        entity_type: "OBJECT".into(),
        subtype: None,
        description: None,
        confidence: 1.0,
        aliases: vec![],
        embedding: None,
        created_at: chrono::Utc::now(),
        updated_at: None,
        metadata: Metadata::new(),
    }
}

#[tokio::test]
async fn preference_filters_forward_together_and_preserve_defaults() {
    let target = Uuid::new_v4();
    for (extra, active, expected_target, expected_time) in [
        (serde_json::json!({}), true, None, None),
        (
            serde_json::json!({"applies_to":null,"as_of":null,"active_only":null}),
            true,
            None,
            None,
        ),
        (
            serde_json::json!({"applies_to":target,"as_of":"2026-01-01T02:00:00+02:00","active_only":false}),
            false,
            Some(target),
            Some(timestamp("2026-01-01T00:00:00Z")),
        ),
    ] {
        let store = Arc::new(RecordingLongTerm::default());
        let mut args = extra;
        args["user_identifier"] = " owner ".into();
        let response = request(
            state_with_long_term(store.clone()),
            "get_preferences_for",
            &args.to_string(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), b"[]");
        assert_eq!(
            store.last_preferences_query(),
            Some((" owner ".into(), active))
        );
        assert_eq!(*store.preference_target.lock().unwrap(), expected_target);
        assert_eq!(*store.preference_as_of.lock().unwrap(), expected_time);
    }
}

#[tokio::test]
async fn preference_filters_reject_wrong_types_before_dependencies() {
    for (key, value, message) in [
        (
            "applies_to",
            serde_json::json!(42),
            "'applies_to' must be a string",
        ),
        (
            "applies_to",
            serde_json::json!([]),
            "'applies_to' must be a string",
        ),
        (
            "as_of",
            serde_json::json!(false),
            "'as_of' must be a string",
        ),
        (
            "active_only",
            serde_json::json!("false"),
            "'active_only' must be a boolean",
        ),
    ] {
        let mut args = serde_json::json!({"user_identifier":"owner"});
        args[key] = value;
        let response = request(state(None, None), "get_preferences_for", &args.to_string()).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()["error"],
            message
        );
    }
}

#[tokio::test]
async fn missing_preference_target_returns_404_without_links_or_creation() {
    let store = Arc::new(RecordingLongTerm::default());
    let target = Uuid::new_v4();
    let missing = Uuid::new_v4();
    store.entities.lock().unwrap().push(target_entity(target));
    let args =
        serde_json::json!({"category":"food", "preference":"tea", "applies_to":[target, missing]});
    let response = request(
        state_with_long_term(store.clone()),
        "add_preference",
        &args.to_string(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()["error"],
        format!("not found: entity {missing}")
    );
    assert!(store.preference.lock().unwrap().is_none());
    assert!(store.applies_to_records().is_empty());
}

#[tokio::test]
async fn preference_target_filter_rejects_malformed_uuid_before_store() {
    let store = Arc::new(RecordingLongTerm::default());
    let response = request(
        state_with_long_term(store.clone()),
        "get_preferences_for",
        r#"{"user_identifier":"owner","applies_to":"not-a-uuid"}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()["error"],
        "'applies_to' must be a UUID"
    );
    assert!(store.last_preferences_query().is_none());
}

#[tokio::test]
async fn preference_time_filter_rejects_malformed_timestamp_before_store() {
    let store = Arc::new(RecordingLongTerm::default());
    let response = request(
        state_with_long_term(store.clone()),
        "get_preferences_for",
        r#"{"user_identifier":"owner","as_of":"yesterday"}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()["error"],
        "'as_of' must be an RFC 3339 timestamp"
    );
    assert!(store.last_preferences_query().is_none());
}

#[tokio::test]
async fn get_preferences_for_returns_the_user_history() {
    let store = Arc::new(RecordingLongTerm::default());
    let preference = Preference {
        id: Uuid::new_v4(),
        category: "food".into(),
        preference: "cafe".into(),
        context: None,
        confidence: 0.8,
        embedding: None,
        created_at: chrono::Utc::now(),
        metadata: Metadata::new(),
        valid_from: Some(timestamp("2026-01-01T00:00:00+00:00")),
        valid_until: Some(timestamp("2026-01-02T00:00:00+00:00")),
    };
    store.set_preferences(vec![preference.clone()]);
    let response = request(
        state_with_long_term(store.clone()),
        "get_preferences_for",
        r#"{"user_identifier":"gm05-user","active_only":false}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        store.last_preferences_query(),
        Some(("gm05-user".into(), false))
    );
    assert_eq!(body[0]["id"], preference.id.to_string());
    assert_eq!(body[0]["confidence"], 0.8);
    assert_eq!(body[0]["valid_from"], "2026-01-01T00:00:00+00:00");
    assert_eq!(body[0]["valid_until"], "2026-01-02T00:00:00+00:00");
}

// Completion and list controls use the public HTTP boundary with recording ports.
#[derive(Default)]
struct ReasoningCalls {
    events: Vec<String>,
    query: Option<TraceQuery>,
    completed: Option<(Option<String>, Option<bool>)>,
}

#[derive(Default)]
struct CompletionStore {
    calls: Mutex<ReasoningCalls>,
    missing: bool,
    no_steps: bool,
    fail_at: Option<&'static str>,
}

impl CompletionStore {
    fn failure(&self, operation: &str) -> MemoryResult<()> {
        if self.fail_at == Some(operation) {
            Err(MemoryError::storage(format!("{operation} failed")))
        } else {
            Ok(())
        }
    }
}

fn completion_trace(id: Uuid) -> ReasoningTrace {
    ReasoningTrace {
        id,
        session_id: "s".into(),
        task: "task".into(),
        task_embedding: None,
        steps: vec![],
        outcome: None,
        success: None,
        started_at: timestamp("2026-01-01T00:00:00Z"),
        completed_at: None,
        metadata: Metadata::new(),
    }
}

#[async_trait]
impl ReasoningStore for CompletionStore {
    async fn create_trace(
        &self,
        _: &TraceRecord,
        _: &OwnerScope,
        _: Option<Uuid>,
    ) -> MemoryResult<ReasoningTrace> {
        unreachable!()
    }
    async fn validate_message_scope(&self, _: Uuid, _: &OwnerScope) -> MemoryResult<()> {
        unreachable!()
    }
    async fn add_conversation_step(
        &self,
        _: Uuid,
        _: Uuid,
        _: &OwnerScope,
        _: &StepRecord,
    ) -> MemoryResult<ReasoningStep> {
        unreachable!()
    }
    async fn link_user_to_trace(&self, _: &str, _: Uuid) -> MemoryResult<()> {
        unreachable!()
    }
    async fn add_step(&self, _: Uuid, _: &StepRecord) -> MemoryResult<ReasoningStep> {
        unreachable!()
    }
    async fn set_step_observation(&self, _: Uuid, _: &str) -> MemoryResult<()> {
        unreachable!()
    }
    async fn steps_without_embedding(&self, id: Uuid) -> MemoryResult<Vec<ReasoningStep>> {
        self.calls.lock().unwrap().events.push("pending".into());
        Ok(vec![ReasoningStep {
            id: Uuid::from_u128(2),
            trace_id: id,
            step_number: 2,
            thought: None,
            action: Some("safe action".into()),
            observation: None,
            tool_calls: vec![],
            embedding: None,
            created_at: timestamp("2026-01-01T00:00:00Z"),
            metadata: Metadata::new(),
        }])
    }
    async fn set_step_embedding(&self, id: Uuid, vector: &[f64]) -> MemoryResult<()> {
        assert_eq!(id, Uuid::from_u128(2));
        assert_eq!(vector, &[1.0]);
        self.calls.lock().unwrap().events.push("embedding".into());
        self.failure("embedding")
    }
    async fn record_tool_call(
        &self,
        _: &ToolCall,
        _: &OwnerScope,
        _: Option<Uuid>,
    ) -> MemoryResult<()> {
        unreachable!()
    }
    async fn complete_trace(
        &self,
        id: Uuid,
        outcome: Option<&str>,
        success: Option<bool>,
    ) -> MemoryResult<Option<ReasoningTrace>> {
        let mut calls = self.calls.lock().unwrap();
        calls.events.push("complete".into());
        self.failure("complete")?;
        if self.missing {
            return Ok(None);
        }
        calls.completed = Some((outcome.map(str::to_owned), success));
        drop(calls);
        let mut trace = completion_trace(id);
        trace.outcome = outcome.map(str::to_owned);
        trace.success = success;
        trace.completed_at = Some(timestamp("2026-01-02T00:00:00Z"));
        Ok(Some(trace))
    }
    async fn get_trace_with_steps(&self, id: Uuid) -> MemoryResult<Option<ReasoningTrace>> {
        self.calls.lock().unwrap().events.push("last".into());
        self.failure("last")?;
        if self.missing {
            return Ok(None);
        }
        let mut trace = completion_trace(id);
        for number in if self.no_steps { vec![] } else { vec![2, 1] } {
            trace.steps.push(ReasoningStep {
                id: Uuid::from_u128(u128::try_from(number).unwrap()),
                trace_id: id,
                step_number: number,
                thought: None,
                action: None,
                observation: None,
                tool_calls: vec![],
                embedding: None,
                created_at: trace.started_at,
                metadata: Metadata::new(),
            });
        }
        Ok(Some(trace))
    }
    async fn set_trace_outcome_fields(
        &self,
        _: Uuid,
        outcome: &muninn_core::model::TraceOutcome,
    ) -> MemoryResult<()> {
        let metrics = serde_json::to_value(&outcome.metrics).unwrap();
        let json = muninn_core::codec::serialize_json(&metrics);
        self.calls.lock().unwrap().events.push(format!(
            "fields:{:?}:{}",
            outcome.error_kind,
            json.as_deref().unwrap_or("null")
        ));
        self.failure("fields")
    }
    async fn list_traces(&self, query: &TraceQuery) -> MemoryResult<Vec<ReasoningTrace>> {
        let mut calls = self.calls.lock().unwrap();
        calls.events.push("list".into());
        calls.query = Some(query.clone());
        drop(calls);
        Ok(vec![])
    }
    async fn tool_stats(&self, _: Option<&str>) -> MemoryResult<Vec<ToolStats>> {
        unreachable!()
    }
    async fn search_traces(
        &self,
        _: &[f64],
        _: i64,
        _: f64,
        _: bool,
    ) -> MemoryResult<Vec<ScoredTrace>> {
        unreachable!()
    }
    async fn search_steps(
        &self,
        _: &[f64],
        _: i64,
        _: f64,
        _: bool,
    ) -> MemoryResult<Vec<ReasoningStepWithContext>> {
        unreachable!()
    }
    async fn link_trace_to_message(&self, _: Uuid, _: Uuid, _: &OwnerScope) -> MemoryResult<bool> {
        unreachable!()
    }
    async fn link_conversation_to_trace(
        &self,
        _: Uuid,
        _: Uuid,
        _: &OwnerScope,
    ) -> MemoryResult<bool> {
        unreachable!()
    }
    async fn link_tool_call_to_message(
        &self,
        _: Uuid,
        _: Uuid,
        _: &OwnerScope,
    ) -> MemoryResult<bool> {
        unreachable!()
    }
    async fn record_touched(&self, id: Uuid, entity: &EntityRef) -> MemoryResult<bool> {
        assert_eq!(id, Uuid::from_u128(2));
        self.calls
            .lock()
            .unwrap()
            .events
            .push(format!("touch:{entity:?}"));
        self.failure("touch")?;
        Ok(true)
    }
    async fn open_conversation_trace(&self, _: Uuid, _: &OwnerScope) -> MemoryResult<Option<Uuid>> {
        unreachable!()
    }
    async fn ensure_conversation_trace(
        &self,
        _: Uuid,
        _: &TraceRecord,
        _: &OwnerScope,
    ) -> MemoryResult<Uuid> {
        unreachable!()
    }
    async fn conversation_steps(
        &self,
        _: Uuid,
        _: &OwnerScope,
    ) -> MemoryResult<Option<Vec<ReasoningStep>>> {
        unreachable!()
    }
    async fn step_context(&self, _: Uuid, _: &OwnerScope) -> MemoryResult<Option<StepContext>> {
        unreachable!()
    }
    async fn entity_provenance(
        &self,
        _: &str,
        _: &OwnerScope,
    ) -> MemoryResult<Vec<ConversationStep>> {
        unreachable!()
    }
}

struct CompletionEmbedder(Arc<CompletionStore>);
#[async_trait]
impl muninn_core::ports::Embedder for CompletionEmbedder {
    fn dimensions(&self) -> usize {
        1
    }
    async fn embed(&self, _: &str) -> MemoryResult<Vec<f64>> {
        unreachable!()
    }
    async fn embed_batch(&self, texts: &[String]) -> MemoryResult<Vec<Vec<f64>>> {
        assert_eq!(texts, &["Action: safe action"]);
        self.0.calls.lock().unwrap().events.push("batch".into());
        self.0.failure("batch")?;
        Ok(vec![vec![1.0]])
    }
}

fn completion_state(store: Arc<CompletionStore>) -> Arc<BridgeState> {
    Arc::new(BridgeState {
        memory: Arc::new(MemoryClient::new(
            MemoryPorts {
                short_term: Arc::new(UnreachableStore::Never),
                long_term: Arc::new(UnreachableLongTerm),
                reasoning: store.clone(),
                embedder: Some(Arc::new(CompletionEmbedder(store))),
                cypher_query: None,
            },
            &MemoryOptions::default(),
        )),
        maintenance: None,
    })
}

async fn completion_request(
    method: &str,
    args: serde_json::Value,
    store: Arc<CompletionStore>,
) -> (StatusCode, serde_json::Value) {
    let response = request(completion_state(store), method, &args.to_string()).await;
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn t2_structured_success_summary_and_last_step_refs() {
    let store = Arc::new(CompletionStore::default());
    let (status, body) = completion_request(
        "complete_trace",
        serde_json::json!({
            "trace_id": Uuid::nil(), "success": true,
            "outcome": {"success":false,"summary":"safe summary","related_entities":[
                {"id":"chosen","name":"ignored","type":"IGNORED"},
                {"name":"Ada","type":"PERSON"}, {"name":"plain"}
            ]}
        }),
        store.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["outcome"], "safe summary");
    assert_eq!(body["success"], false);
    assert_eq!(
        body,
        serde_json::json!({
            "id":Uuid::nil(),"session_id":"s","task":"task","steps":[],
            "outcome":"safe summary","success":false,
            "started_at":"2026-01-01T00:00:00+00:00","completed_at":"2026-01-02T00:00:00+00:00"
        })
    );
    let calls = store.calls.lock().unwrap();
    assert_eq!(
        calls.completed,
        Some((Some("safe summary".into()), Some(false)))
    );
    assert_eq!(
        calls.events,
        [
            "complete",
            "last",
            "touch:Id(\"chosen\")",
            "touch:NameType { name: \"Ada\", entity_type: \"PERSON\" }",
            "touch:Name(\"plain\")"
        ]
    );
}

#[tokio::test]
async fn t2_missing_trace_checks_not_found_after_structured_writes() {
    let store = Arc::new(CompletionStore {
        missing: true,
        ..Default::default()
    });
    let (status, body) = completion_request("complete_trace", serde_json::json!({
        "trace_id":Uuid::nil(),"outcome":{"success":false,"summary":"absent","error_kind":"missing","related_entities":[{"name":"x"}]}
    }), store.clone()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        body,
        serde_json::json!({"error":format!("not found: trace {}", Uuid::nil())})
    );
    let calls = store.calls.lock().unwrap();
    assert_eq!(
        calls.events,
        ["complete", "fields:Some(\"missing\"):null", "last"]
    );
    assert!(calls.completed.is_none());
    drop(calls);
}

#[tokio::test]
async fn t2_structured_error_and_metrics_are_persisted_after_completion() {
    let store = Arc::new(CompletionStore::default());
    let (status, body) = completion_request("complete_trace", serde_json::json!({
        "trace_id":Uuid::nil(),"generate_step_embeddings":true,
        "outcome":{"success":true,"summary":"done","error_kind":"timeout","metrics":{"latency_ms":2.5,"tools":2}}
    }), store.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["outcome"], "done");
    assert_eq!(body["success"], true);
    assert!(body.get("metadata").is_none());
    assert_eq!(
        store.calls.lock().unwrap().events,
        [
            "pending",
            "batch",
            "embedding",
            "complete",
            "fields:Some(\"timeout\"):{\"latency_ms\": 2.5, \"tools\": 2.0}"
        ]
    );
}

#[tokio::test]
async fn t2_malformed_structured_outcomes_have_no_side_effects() {
    for (outcome, error) in [
        (
            serde_json::json!({"summary":"s"}),
            "missing argument 'success'",
        ),
        (
            serde_json::json!({"success":true}),
            "missing argument 'summary'",
        ),
        (
            serde_json::json!({"success":true,"summary":4}),
            "'summary' must be a string",
        ),
        (
            serde_json::json!({"success":"true","summary":"s"}),
            "'success' must be a boolean",
        ),
        (
            serde_json::json!({"success":true,"summary":"s","error_kind":false}),
            "'error_kind' must be a string",
        ),
        (
            serde_json::json!({"success":true,"summary":"s","metrics":{"n":"bad"}}),
            "outcome metrics must be finite numbers",
        ),
        (
            serde_json::json!({"success":true,"summary":"s","related_entities":[{}]}),
            "missing argument 'name'",
        ),
        (
            serde_json::json!({"success":true,"summary":"s","related_entities":[{"name":""}]}),
            "entity reference requires a nonempty name or id",
        ),
    ] {
        let store = Arc::new(CompletionStore::default());
        let (status, body) = completion_request(
            "complete_trace",
            serde_json::json!({
                "trace_id":Uuid::nil(),"generate_step_embeddings":true,"outcome":outcome
            }),
            store.clone(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body, serde_json::json!({"error":error}));
        let calls = store.calls.lock().unwrap();
        assert!(calls.events.is_empty());
        assert!(calls.completed.is_none());
        assert!(calls.query.is_none());
        drop(calls);
    }
}

#[tokio::test]
async fn t2_completion_can_embed_deferred_steps() {
    let store = Arc::new(CompletionStore::default());
    let (status, body) = completion_request("complete_trace", serde_json::json!({
        "trace_id": Uuid::nil(), "outcome":"done", "success":true, "generate_step_embeddings":true
    }), store.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["outcome"], "done");
    assert_eq!(
        store.calls.lock().unwrap().events,
        ["pending", "batch", "embedding", "complete"]
    );
}

#[tokio::test]
async fn t2_list_forwards_all_controls() {
    let store = Arc::new(CompletionStore::default());
    let (status, body) = completion_request("list_traces", serde_json::json!({
        "session_id":"s", "success_only":false, "since":"2026-01-01T02:00:00+02:00",
        "until":"2026-01-03T00:00:00Z", "limit":2,"offset":3,"order_by":"completed_at","order_dir":"asc"
    }), store.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, serde_json::json!([]));
    let calls = store.calls.lock().unwrap();
    let query = calls.query.as_ref().unwrap();
    assert_eq!(query.session_id.as_deref(), Some("s"));
    assert_eq!(query.success, Some(false));
    assert_eq!(query.since, Some(timestamp("2026-01-01T00:00:00Z")));
    assert_eq!(query.until, Some(timestamp("2026-01-03T00:00:00Z")));
    assert_eq!((query.limit, query.offset), (2, 3));
    assert_eq!(query.order_by, muninn_core::ports::TraceOrder::CompletedAt);
    assert!(query.ascending);
    drop(calls);
}

#[tokio::test]
async fn t2_rejected_list_controls_do_not_reach_dependencies() {
    for (key, value, error) in [
        (
            "success_only",
            serde_json::json!("false"),
            "'success_only' must be a boolean",
        ),
        (
            "since",
            serde_json::json!("yesterday"),
            "'since' must be an RFC 3339 timestamp",
        ),
        ("until", serde_json::json!(true), "'until' must be a string"),
        (
            "offset",
            serde_json::json!(1.5),
            "'offset' must be an integer",
        ),
        (
            "offset",
            serde_json::json!(-1),
            "'offset' must be nonnegative",
        ),
        (
            "limit",
            serde_json::json!(-1),
            "'limit' must be nonnegative",
        ),
        (
            "order_by",
            serde_json::json!("task"),
            "'order_by' must be 'started_at' or 'completed_at'",
        ),
        (
            "order_dir",
            serde_json::json!("ASC"),
            "'order_dir' must be 'asc' or 'desc'",
        ),
    ] {
        let store = Arc::new(CompletionStore::default());
        let mut args = serde_json::json!({});
        args[key] = value;
        let (status, body) = completion_request("list_traces", args, store.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{key}");
        assert_eq!(body, serde_json::json!({"error":error}));
        let calls = store.calls.lock().unwrap();
        assert!(calls.events.is_empty());
        assert!(calls.query.is_none());
        assert!(calls.completed.is_none());
        drop(calls);
    }
}

#[tokio::test]
async fn t2_invalid_embedding_switch_is_rejected_before_completion() {
    let store = Arc::new(CompletionStore::default());
    let (status, body) = completion_request(
        "complete_trace",
        serde_json::json!({
            "trace_id":Uuid::nil(),"outcome":"done","generate_step_embeddings":"true"
        }),
        store.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body,
        serde_json::json!({"error":"'generate_step_embeddings' must be a boolean"})
    );
    assert!(store.calls.lock().unwrap().events.is_empty());
}

#[tokio::test]
async fn t2_structured_empty_fields_and_no_steps_preserve_runtime_boundaries() {
    for (extra, expected) in [
        (serde_json::json!({}), vec!["complete"]),
        (
            serde_json::json!({"error_kind":null,"metrics":{},"related_entities":[]}),
            vec!["complete"],
        ),
        (
            serde_json::json!({"error_kind":""}),
            vec!["complete", "fields:Some(\"\"):null"],
        ),
        (
            serde_json::json!({"metrics":{"n":0}}),
            vec!["complete", "fields:None:{\"n\": 0.0}"],
        ),
        (
            serde_json::json!({"related_entities":[{"name":"x"}]}),
            vec!["complete", "last"],
        ),
    ] {
        let store = Arc::new(CompletionStore {
            no_steps: true,
            ..Default::default()
        });
        let mut outcome = serde_json::json!({"success":true,"summary":""});
        outcome
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let (status, body) = completion_request(
            "complete_trace",
            serde_json::json!({"trace_id":Uuid::nil(),"outcome":outcome}),
            store.clone(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["outcome"], "");
        assert_eq!(body["success"], true);
        assert_eq!(store.calls.lock().unwrap().events, expected);
    }
}

#[tokio::test]
async fn t2_completion_errors_propagate_without_later_writes() {
    let all = [
        "pending",
        "batch",
        "embedding",
        "complete",
        "fields:Some(\"e\"):null",
        "last",
        "touch:Name(\"x\")",
    ];
    for (failure, end) in [
        ("batch", 2),
        ("embedding", 3),
        ("complete", 4),
        ("fields", 5),
        ("last", 6),
        ("touch", 7),
    ] {
        let store = Arc::new(CompletionStore {
            fail_at: Some(failure),
            ..Default::default()
        });
        let (status, body) = completion_request("complete_trace", serde_json::json!({
            "trace_id":Uuid::nil(),"generate_step_embeddings":true,
            "outcome":{"success":false,"summary":"failed","error_kind":"e","related_entities":[{"name":"x"}]}
        }), store.clone()).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            body,
            serde_json::json!({"error":format!("storage error: {failure} failed")})
        );
        let calls = store.calls.lock().unwrap();
        assert_eq!(calls.events, all[..end]);
        assert_eq!(calls.completed.is_some(), end > 4);
    }
}

#[tokio::test]
async fn t2_all_malformed_structured_containers_are_rejected() {
    for (field, value, error) in [
        (
            "metrics",
            serde_json::json!([]),
            "'metrics' must be an object",
        ),
        (
            "metrics",
            serde_json::Value::Null,
            "'metrics' must be an object",
        ),
        (
            "related_entities",
            serde_json::Value::Null,
            "'touched_entities' must be an array",
        ),
        (
            "related_entities",
            serde_json::json!({}),
            "'touched_entities' must be an array",
        ),
        (
            "related_entities",
            serde_json::json!([7]),
            "entity reference must be an object",
        ),
        (
            "related_entities",
            serde_json::json!([{"id":7}]),
            "'id' must be a string",
        ),
        (
            "related_entities",
            serde_json::json!([{"id":"id","type":7}]),
            "'type' must be a string",
        ),
    ] {
        let store = Arc::new(CompletionStore::default());
        let mut outcome = serde_json::json!({"success":true,"summary":"s"});
        outcome[field] = value;
        let (status, body) = completion_request("complete_trace", serde_json::json!({"trace_id":Uuid::nil(),"outcome":outcome,"generate_step_embeddings":true}), store.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body, serde_json::json!({"error":error}));
        assert!(store.calls.lock().unwrap().events.is_empty());
    }
}

#[tokio::test]
async fn t2_list_null_defaults_and_zero_pagination_are_forwarded() {
    for args in [
        serde_json::json!({}),
        serde_json::json!({"session_id":null,"success_only":null,"since":null,"until":null,
            "limit":null,"offset":null,"order_by":null,"order_dir":null}),
        serde_json::json!({"success_only":true,"limit":0,"offset":0,
            "order_by":"started_at","order_dir":"desc"}),
    ] {
        let store = Arc::new(CompletionStore::default());
        let (status, body) = completion_request("list_traces", args.clone(), store.clone()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, serde_json::json!([]));
        let query = store.calls.lock().unwrap().query.clone().unwrap();
        assert!(query.session_id.is_none() && query.since.is_none() && query.until.is_none());
        assert_eq!(query.success, args["success_only"].as_bool());
        assert_eq!(query.limit, args["limit"].as_i64().unwrap_or(100));
        assert_eq!(query.offset, 0);
        assert_eq!(query.order_by, muninn_core::ports::TraceOrder::StartedAt);
        assert!(!query.ascending);
    }
}

#[tokio::test]
async fn t2_completion_validates_uuid_and_success_before_embeddings() {
    for (key, value, error) in [
        (
            "trace_id",
            serde_json::json!("bad"),
            "'trace_id' must be a UUID",
        ),
        (
            "success",
            serde_json::json!(1),
            "'success' must be a boolean",
        ),
    ] {
        let store = Arc::new(CompletionStore::default());
        let mut args = serde_json::json!({"trace_id":Uuid::nil(),"outcome":"legacy",
            "generate_step_embeddings":true});
        args[key] = value;
        let (status, body) = completion_request("complete_trace", args, store.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body, serde_json::json!({"error":error}));
        assert!(store.calls.lock().unwrap().events.is_empty());
    }
}

#[tokio::test]
async fn t2_preserves_legacy_completion_and_list_defaults() {
    for outcome in [serde_json::json!("done"), serde_json::Value::Null] {
        for flag in [
            None,
            Some(serde_json::json!(false)),
            Some(serde_json::Value::Null),
        ] {
            let store = Arc::new(CompletionStore::default());
            let mut args =
                serde_json::json!({"trace_id":Uuid::nil(),"outcome":outcome,"success":false});
            if let Some(flag) = flag {
                args["generate_step_embeddings"] = flag;
            }
            let (status, body) = completion_request("complete_trace", args, store.clone()).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(body["outcome"], outcome);
            assert_eq!(body["success"], false);
            assert_eq!(store.calls.lock().unwrap().events, ["complete"]);
        }
    }
    let store = Arc::new(CompletionStore::default());
    let (status, body) = completion_request(
        "list_traces",
        serde_json::json!({"session_id":"old","limit":7}),
        store.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, serde_json::json!([]));
    let calls = store.calls.lock().unwrap();
    let query = calls.query.as_ref().unwrap();
    assert_eq!(query.session_id.as_deref(), Some("old"));
    assert_eq!(query.limit, 7);
    assert_eq!(query.offset, 0);
    assert!(query.success.is_none() && query.since.is_none() && query.until.is_none());
    assert_eq!(query.order_by, muninn_core::ports::TraceOrder::StartedAt);
    assert!(!query.ascending);
    drop(calls);
}
