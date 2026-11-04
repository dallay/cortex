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
            created_at: chrono::Utc::now(),
            metadata: record.metadata.clone(),
            embedding: record.embedding.clone(),
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
        _: &str,
        _: Option<Uuid>,
        _: &OwnerScope,
        _: Option<i64>,
    ) -> MemoryResult<Option<Conversation>> {
        unreachable!()
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

/// Long-term store that records what the bridge actually forwards.
#[derive(Default)]
struct RecordingLongTerm {
    fact: Mutex<Option<Fact>>,
    preference: Mutex<Option<Preference>>,
    applies_to: Mutex<Vec<(Uuid, Uuid)>>,
    relationship: Mutex<Option<Relationship>>,
    facts: Mutex<Vec<Fact>>,
    preferences: Mutex<Vec<Preference>>,
    supersessions: Mutex<Vec<(Uuid, Uuid)>>,
    preferences_query: Mutex<Option<(String, bool)>>,
    preference_target: Mutex<Option<Uuid>>,
    preference_as_of: Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    entities: Mutex<Vec<Entity>>,
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

    fn stored_preference(&self) -> Preference {
        self.preference
            .lock()
            .unwrap()
            .clone()
            .expect("preference not stored")
    }
}

fn timestamp(text: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&chrono::Utc)
}

#[async_trait]
impl LongTermStore for RecordingLongTerm {
    async fn upsert_entity(&self, _: &Entity) -> MemoryResult<Entity> {
        unreachable!()
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
