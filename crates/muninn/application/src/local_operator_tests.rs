use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use muninn_core::extraction::{EntityExtractor, ExtractOptions, ExtractionResult};
use muninn_core::model::*;
use muninn_core::ports::*;
use muninn_core::{MemoryError, MemoryResult};
use uuid::Uuid;

use crate::{MemoryClient, MemoryOptions, MemoryPorts};

const OWNER: &str = " owner from composition ";
const SESSION: &str = "support-session";
const CONVERSATION: Uuid = Uuid::from_u128(1);

#[test]
fn local_operator_is_disabled_by_default_at_all_four_entries() {
    for client in [
        MemoryClient::new(ports(Arc::new(Store::default())), &MemoryOptions::default()).unwrap(),
        MemoryClient::read_only(ports(Arc::new(Store::default())), &MemoryOptions::default()),
    ] {
        let local = client.local_operator();
        for result in [
            client.graph(),
            local.short_term().client(),
            local.long_term().client(),
            local.reasoning().client(),
        ] {
            assert!(matches!(result, Err(MemoryError::NotSupported(_))));
        }
    }
}

#[tokio::test]
async fn local_operator_routes_share_the_same_fixed_owner() {
    let store = Arc::new(Store::default());
    let client = MemoryClient::new(ports(store.clone()), &MemoryOptions::default())
        .unwrap()
        .with_extractor(store.clone())
        .with_local_operator(OWNER)
        .unwrap();
    let local = client.local_operator();
    for access in [
        client.graph(),
        local.short_term().client(),
        local.long_term().client(),
        local.reasoning().client(),
    ] {
        let operator = access.unwrap();
        assert_eq!(
            operator
                .get_conversation(SESSION, Some(5))
                .await
                .unwrap()
                .id,
            CONVERSATION
        );
        let message = operator
            .add_message(SESSION, MessageRole::User, "hello")
            .await
            .unwrap();
        assert_eq!(message.content, "hello");
        assert_eq!(message.conversation_id, Some(CONVERSATION));
        assert!(operator.get_preferences(true).await.unwrap().is_empty());
        let pref = operator.add_preference("editor", "spaces").await.unwrap();
        assert_eq!(pref.preference, "spaces");
        let trace = operator.get_conversation_trace(CONVERSATION).await.unwrap();
        assert_eq!(trace.conversation_id, CONVERSATION);
        assert_eq!(
            operator.start_trace(SESSION, "review").await.unwrap().task,
            "review"
        );
    }
    assert_eq!(
        *store.calls.lock().unwrap(),
        [
            "read conversation",
            "lookup conversation for append",
            "ensure conversation",
            "append message",
            "read preferences",
            "write preference",
            "read conversation steps",
            "create trace",
        ]
        .repeat(4),
    );
}

#[tokio::test]
async fn local_operator_read_only_client_rejects_all_writes_before_storage() {
    let store = Arc::new(Store::default());
    let client = MemoryClient::read_only(ports(store.clone()), &MemoryOptions::default())
        .with_local_operator(OWNER)
        .unwrap();
    let local = client.local_operator();
    for access in [
        client.graph(),
        local.short_term().client(),
        local.long_term().client(),
        local.reasoning().client(),
    ] {
        let operator = access.unwrap();
        assert!(matches!(
            operator
                .add_message(SESSION, MessageRole::User, "hello")
                .await,
            Err(MemoryError::NotSupported(_))
        ));
        assert!(matches!(
            operator.add_preference("editor", "spaces").await,
            Err(MemoryError::NotSupported(_))
        ));
        assert!(matches!(
            operator.start_trace(SESSION, "review").await,
            Err(MemoryError::NotSupported(_))
        ));
    }
    assert!(store.calls.lock().unwrap().is_empty());
    assert!(client
        .graph()
        .unwrap()
        .get_preferences(true)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(*store.calls.lock().unwrap(), ["read preferences"]);
}

#[test]
fn local_operator_requires_a_non_blank_owner_and_can_only_be_bound_once() {
    let new =
        || MemoryClient::new(ports(Arc::new(Store::default())), &MemoryOptions::default()).unwrap();
    for owner in ["", " \t\n"] {
        assert!(matches!(
            new().with_local_operator(owner),
            Err(MemoryError::InvalidInput(_))
        ));
    }
    assert!(new()
        .with_local_operator(OWNER)
        .unwrap()
        .with_local_operator("second owner")
        .is_err());
}

#[derive(Default)]
struct Store {
    calls: Mutex<Vec<&'static str>>,
}

impl Store {
    fn scoped_call(&self, call: &'static str, owner: Option<&str>) {
        assert_eq!(owner, Some(OWNER));
        self.calls.lock().unwrap().push(call);
    }
}

fn ports(store: Arc<Store>) -> MemoryPorts {
    MemoryPorts {
        short_term: store.clone(),
        long_term: store.clone(),
        reasoning: store.clone(),
        embedder: Some(store),
        cypher_query: None,
    }
}

#[async_trait]
impl Embedder for Store {
    fn dimensions(&self) -> usize {
        1
    }

    async fn embed(&self, _: &str) -> MemoryResult<Vec<f64>> {
        unreachable!("the local operator must not call the embedder")
    }
}

#[async_trait]
impl EntityExtractor for Store {
    async fn extract(&self, _: &str, _: &ExtractOptions) -> MemoryResult<ExtractionResult> {
        unreachable!("the local operator must not call the extractor")
    }
}

#[async_trait]
impl ShortTermStore for Store {
    async fn ensure_conversation(
        &self,
        session: &str,
        id: Option<Uuid>,
        scope: &OwnerScope,
    ) -> MemoryResult<Uuid> {
        self.scoped_call("ensure conversation", scope.as_str());
        assert_eq!(session, SESSION);
        assert_eq!(id, None);
        Ok(CONVERSATION)
    }
    async fn find_by_operation_key(
        &self,
        _: Uuid,
        _: &OwnerScope,
        _: &str,
    ) -> MemoryResult<Option<KeyedMessage>> {
        unreachable!("unexpected find_by_operation_key")
    }
    async fn append_message(
        &self,
        id: Uuid,
        scope: &OwnerScope,
        record: &MessageRecord,
    ) -> MemoryResult<Message> {
        self.scoped_call("append message", scope.as_str());
        assert_eq!(id, CONVERSATION);
        assert!(record.embedding.is_none());
        Ok(Message {
            id: record.id,
            role: record.role.parse()?,
            content: record.content.clone(),
            conversation_id: Some(id),
            created_at: chrono::Utc::now(),
            embedding: None,
            metadata: record.metadata.clone(),
        })
    }
    async fn append_messages(
        &self,
        _: Uuid,
        _: &OwnerScope,
        _: &[MessageRecord],
    ) -> MemoryResult<Vec<Message>> {
        unreachable!("unexpected append_messages")
    }
    async fn get_conversation(
        &self,
        session: &str,
        id: Option<Uuid>,
        scope: &OwnerScope,
        limit: Option<i64>,
    ) -> MemoryResult<Option<Conversation>> {
        assert_eq!(session, SESSION);
        assert_eq!(id, None);
        if limit == Some(1) {
            self.scoped_call("lookup conversation for append", scope.as_str());
            return Ok(None);
        }
        self.scoped_call("read conversation", scope.as_str());
        assert_eq!(limit, Some(5));
        Ok(Some(Conversation {
            id: CONVERSATION,
            ..Conversation::empty(session)
        }))
    }
    async fn get_conversation_by_id(
        &self,
        _: Uuid,
        _: &OwnerScope,
    ) -> MemoryResult<Option<Conversation>> {
        unreachable!("unexpected get_conversation_by_id")
    }
    async fn append_observation(&self, _: &Observation, _: &OwnerScope) -> MemoryResult<()> {
        unreachable!("unexpected append_observation")
    }
    async fn list_observations(
        &self,
        _: Uuid,
        _: &OwnerScope,
        _: i64,
        _: i64,
    ) -> MemoryResult<Vec<Observation>> {
        unreachable!("unexpected list_observations")
    }
    async fn list_conversations(&self, _: &OwnerScope, _: i64) -> MemoryResult<Vec<Conversation>> {
        unreachable!("unexpected list_conversations")
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
        unreachable!("unexpected search_messages")
    }
    async fn list_sessions(
        &self,
        _: &SessionQuery,
        _: &OwnerScope,
    ) -> MemoryResult<Vec<SessionInfo>> {
        unreachable!("unexpected list_sessions")
    }
    async fn delete_message(&self, _: Uuid, _: &OwnerScope, _: bool) -> MemoryResult<bool> {
        unreachable!("unexpected delete_message")
    }
    async fn clear_session(&self, _: &str, _: &OwnerScope) -> MemoryResult<()> {
        unreachable!("unexpected clear_session")
    }
    async fn delete_conversation(&self, _: Uuid, _: &OwnerScope) -> MemoryResult<()> {
        unreachable!("unexpected delete_conversation")
    }
    async fn summary_entities(&self, _: &str, _: &OwnerScope) -> MemoryResult<Vec<String>> {
        unreachable!("unexpected summary_entities")
    }
}

#[async_trait]
impl LongTermStore for Store {
    fn supports_atomic_preference_write(&self) -> bool {
        true
    }
    async fn upsert_entity(&self, _: &Entity) -> MemoryResult<Entity> {
        unreachable!("unexpected upsert_entity")
    }
    async fn get_entity(&self, _: Uuid) -> MemoryResult<Option<Entity>> {
        unreachable!("unexpected get_entity")
    }
    async fn get_entity_by_name(&self, _: &str) -> MemoryResult<Option<Entity>> {
        unreachable!("unexpected get_entity_by_name")
    }
    async fn search_entities(
        &self,
        _: &[f64],
        _: i64,
        _: f64,
    ) -> MemoryResult<Vec<Scored<Entity>>> {
        unreachable!("unexpected search_entities")
    }
    async fn list_entities(&self, _: Option<&str>, _: i64) -> MemoryResult<Vec<Entity>> {
        unreachable!("unexpected list_entities")
    }
    async fn update_entity(&self, _: Uuid, _: &EntityUpdate) -> MemoryResult<Option<Entity>> {
        unreachable!("unexpected update_entity")
    }
    async fn delete_entity(&self, _: Uuid) -> MemoryResult<bool> {
        unreachable!("unexpected delete_entity")
    }
    async fn related_entities(&self, _: Uuid, _: u8) -> MemoryResult<Vec<RelatedEntity>> {
        unreachable!("unexpected related_entities")
    }
    async fn upsert_relationship(&self, _: &Relationship) -> MemoryResult<Option<Relationship>> {
        unreachable!("unexpected upsert_relationship")
    }
    async fn merge_entities(&self, _: Uuid, _: Uuid) -> MemoryResult<Option<(Entity, Entity)>> {
        unreachable!("unexpected merge_entities")
    }
    async fn link_same_as(&self, _: Uuid, _: Uuid, _: f64, _: &str, _: &str) -> MemoryResult<()> {
        unreachable!("unexpected link_same_as")
    }
    async fn set_entity_feedback(&self, _: Uuid, _: f64, _: bool) -> MemoryResult<bool> {
        unreachable!("unexpected set_entity_feedback")
    }
    async fn link_message_to_entity(&self, _: Uuid, _: Uuid, _: f64) -> MemoryResult<bool> {
        unreachable!("unexpected link_message_to_entity")
    }
    async fn entity_mentions(&self, _: Uuid) -> MemoryResult<Option<Vec<EntityMention>>> {
        unreachable!("unexpected entity_mentions")
    }
    async fn entity_graph(&self, _: i64) -> MemoryResult<EntityGraph> {
        unreachable!("unexpected entity_graph")
    }
    async fn create_preference(&self, _: &Preference) -> MemoryResult<Preference> {
        unreachable!("unexpected create_preference")
    }
    async fn find_duplicate_preference(
        &self,
        _: &[f64],
        _: &str,
        _: f64,
        _: Option<&str>,
    ) -> MemoryResult<Option<Preference>> {
        unreachable!("unexpected find_duplicate_preference")
    }
    async fn link_user_to_preference(&self, _: &str, _: Uuid) -> MemoryResult<()> {
        unreachable!("unexpected link_user_to_preference")
    }
    async fn link_preference_to_entity(&self, _: Uuid, _: Uuid) -> MemoryResult<()> {
        unreachable!("unexpected link_preference_to_entity")
    }
    async fn supersede_preference(&self, _: Uuid, _: Uuid) -> MemoryResult<()> {
        unreachable!("unexpected supersede_preference")
    }
    async fn preferences_for(
        &self,
        owner: &str,
        active_only: bool,
    ) -> MemoryResult<Vec<Preference>> {
        self.scoped_call("read preferences", Some(owner));
        assert!(active_only);
        Ok(vec![])
    }
    async fn write_preference(
        &self,
        pref: &Preference,
        owner: Option<&str>,
        applies: &[Uuid],
        about: &[Uuid],
        _: f64,
    ) -> MemoryResult<Preference> {
        self.scoped_call("write preference", owner);
        assert!(applies.is_empty() && about.is_empty());
        assert!(pref.embedding.is_none());
        Ok(pref.clone())
    }
    async fn search_preferences(
        &self,
        _: &[f64],
        _: i64,
        _: f64,
        _: Option<&str>,
    ) -> MemoryResult<Vec<Scored<Preference>>> {
        unreachable!("unexpected search_preferences")
    }
    async fn preferences_by_category(
        &self,
        _: &str,
        _: i64,
        _: Option<&str>,
    ) -> MemoryResult<Vec<Preference>> {
        unreachable!("unexpected preferences_by_category")
    }
    async fn create_fact(&self, _: &Fact) -> MemoryResult<Fact> {
        unreachable!("unexpected create_fact")
    }
    async fn find_duplicate_fact(
        &self,
        _: &[f64],
        _: &str,
        _: &str,
        _: f64,
    ) -> MemoryResult<Option<Fact>> {
        unreachable!("unexpected find_duplicate_fact")
    }
    async fn update_fact_confidence(&self, _: Uuid, _: f64, _: &str) -> MemoryResult<()> {
        unreachable!("unexpected update_fact_confidence")
    }
    async fn search_facts(&self, _: &[f64], _: i64, _: f64) -> MemoryResult<Vec<Scored<Fact>>> {
        unreachable!("unexpected search_facts")
    }
    async fn facts_by_subject(&self, _: &str, _: i64) -> MemoryResult<Vec<Fact>> {
        unreachable!("unexpected facts_by_subject")
    }
}

#[async_trait]
impl ReasoningStore for Store {
    async fn create_trace(
        &self,
        record: &TraceRecord,
        scope: &OwnerScope,
        message: Option<Uuid>,
    ) -> MemoryResult<ReasoningTrace> {
        self.scoped_call("create trace", scope.as_str());
        assert_eq!(record.session_id, SESSION);
        assert!(message.is_none() && record.task_embedding.is_none());
        Ok(ReasoningTrace {
            id: record.id,
            session_id: record.session_id.clone(),
            task: record.task.clone(),
            task_embedding: None,
            steps: vec![],
            outcome: None,
            success: None,
            started_at: chrono::Utc::now(),
            completed_at: None,
            metadata: record.metadata.clone(),
        })
    }
    async fn validate_message_scope(&self, _: Uuid, _: &OwnerScope) -> MemoryResult<()> {
        unreachable!("unexpected validate_message_scope")
    }
    async fn add_conversation_step(
        &self,
        _: Uuid,
        _: Uuid,
        _: &OwnerScope,
        _: &StepRecord,
    ) -> MemoryResult<ReasoningStep> {
        unreachable!("unexpected add_conversation_step")
    }
    async fn link_user_to_trace(&self, _: &str, _: Uuid) -> MemoryResult<()> {
        unreachable!("unexpected link_user_to_trace")
    }
    async fn add_step(&self, _: Uuid, _: &StepRecord) -> MemoryResult<ReasoningStep> {
        unreachable!("unexpected add_step")
    }
    async fn set_step_observation(&self, _: Uuid, _: &str) -> MemoryResult<()> {
        unreachable!("unexpected set_step_observation")
    }
    async fn steps_without_embedding(&self, _: Uuid) -> MemoryResult<Vec<ReasoningStep>> {
        unreachable!("unexpected steps_without_embedding")
    }
    async fn set_step_embedding(&self, _: Uuid, _: &[f64]) -> MemoryResult<()> {
        unreachable!("unexpected set_step_embedding")
    }
    async fn record_tool_call(
        &self,
        _: &ToolCall,
        _: &OwnerScope,
        _: Option<Uuid>,
    ) -> MemoryResult<()> {
        unreachable!("unexpected record_tool_call")
    }
    async fn complete_trace(
        &self,
        _: Uuid,
        _: Option<&str>,
        _: Option<bool>,
    ) -> MemoryResult<Option<ReasoningTrace>> {
        unreachable!("unexpected complete_trace")
    }
    async fn get_trace_with_steps(&self, _: Uuid) -> MemoryResult<Option<ReasoningTrace>> {
        unreachable!("unexpected get_trace_with_steps")
    }
    async fn list_traces(&self, _: &TraceQuery) -> MemoryResult<Vec<ReasoningTrace>> {
        unreachable!("unexpected list_traces")
    }
    async fn tool_stats(&self, _: Option<&str>) -> MemoryResult<Vec<ToolStats>> {
        unreachable!("unexpected tool_stats")
    }
    async fn search_traces(
        &self,
        _: &[f64],
        _: i64,
        _: f64,
        _: bool,
    ) -> MemoryResult<Vec<ScoredTrace>> {
        unreachable!("unexpected search_traces")
    }
    async fn search_steps(
        &self,
        _: &[f64],
        _: i64,
        _: f64,
        _: bool,
    ) -> MemoryResult<Vec<ReasoningStepWithContext>> {
        unreachable!("unexpected search_steps")
    }
    async fn link_trace_to_message(&self, _: Uuid, _: Uuid, _: &OwnerScope) -> MemoryResult<bool> {
        unreachable!("unexpected link_trace_to_message")
    }
    async fn link_conversation_to_trace(
        &self,
        _: Uuid,
        _: Uuid,
        _: &OwnerScope,
    ) -> MemoryResult<bool> {
        unreachable!("unexpected link_conversation_to_trace")
    }
    async fn link_tool_call_to_message(
        &self,
        _: Uuid,
        _: Uuid,
        _: &OwnerScope,
    ) -> MemoryResult<bool> {
        unreachable!("unexpected link_tool_call_to_message")
    }
    async fn record_touched(&self, _: Uuid, _: &EntityRef) -> MemoryResult<bool> {
        unreachable!("unexpected record_touched")
    }
    async fn open_conversation_trace(&self, _: Uuid, _: &OwnerScope) -> MemoryResult<Option<Uuid>> {
        unreachable!("unexpected open_conversation_trace")
    }
    async fn ensure_conversation_trace(
        &self,
        _: Uuid,
        _: &TraceRecord,
        _: &OwnerScope,
    ) -> MemoryResult<Uuid> {
        unreachable!("unexpected ensure_conversation_trace")
    }
    async fn conversation_steps(
        &self,
        id: Uuid,
        scope: &OwnerScope,
    ) -> MemoryResult<Option<Vec<ReasoningStep>>> {
        self.scoped_call("read conversation steps", scope.as_str());
        assert_eq!(id, CONVERSATION);
        Ok(Some(vec![]))
    }
    async fn step_context(&self, _: Uuid, _: &OwnerScope) -> MemoryResult<Option<StepContext>> {
        unreachable!("unexpected step_context")
    }
    async fn entity_provenance(
        &self,
        _: &str,
        _: &OwnerScope,
    ) -> MemoryResult<Vec<ConversationStep>> {
        unreachable!("unexpected entity_provenance")
    }
}

struct AtomicShortTermStore {
    fail_next_atomic: std::sync::atomic::AtomicBool,
    atomic_calls: std::sync::atomic::AtomicUsize,
    append_calls: std::sync::atomic::AtomicUsize,
    committed_messages: Mutex<Vec<MessageRecord>>,
    committed_batches: Mutex<Vec<muninn_core::ports::AtomicExtractionBatch>>,
}

impl Default for AtomicShortTermStore {
    fn default() -> Self {
        Self {
            fail_next_atomic: std::sync::atomic::AtomicBool::new(false),
            atomic_calls: std::sync::atomic::AtomicUsize::new(0),
            append_calls: std::sync::atomic::AtomicUsize::new(0),
            committed_messages: Mutex::new(Vec::new()),
            committed_batches: Mutex::new(Vec::new()),
        }
    }
}

fn atomic_record_to_message(
    record: &MessageRecord,
    conversation_id: Uuid,
) -> MemoryResult<Message> {
    Ok(Message {
        id: record.id,
        role: record.role.parse()?,
        content: record.content.clone(),
        conversation_id: Some(conversation_id),
        created_at: record.timestamp.unwrap_or_else(chrono::Utc::now),
        embedding: record.embedding.clone(),
        metadata: record.metadata.clone(),
    })
}

#[async_trait]
impl ShortTermStore for AtomicShortTermStore {
    async fn ensure_conversation(
        &self,
        _: &str,
        _: Option<Uuid>,
        _: &OwnerScope,
    ) -> MemoryResult<Uuid> {
        Ok(CONVERSATION)
    }
    async fn find_by_operation_key(
        &self,
        conversation_id: Uuid,
        _: &OwnerScope,
        operation_key: &str,
    ) -> MemoryResult<Option<KeyedMessage>> {
        let committed = self.committed_messages.lock().unwrap();
        let found = committed
            .iter()
            .find(|r| r.operation_key.as_deref() == Some(operation_key));
        match found {
            Some(record) => Ok(Some(KeyedMessage {
                message: atomic_record_to_message(record, conversation_id)?,
                operation_payload: record.operation_payload.clone(),
            })),
            None => Ok(None),
        }
    }
    async fn append_message(
        &self,
        conversation_id: Uuid,
        _: &OwnerScope,
        record: &MessageRecord,
    ) -> MemoryResult<Message> {
        self.append_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.committed_messages.lock().unwrap().push(record.clone());
        atomic_record_to_message(record, conversation_id)
    }
    async fn append_message_atomic(
        &self,
        conversation_id: Uuid,
        _: &OwnerScope,
        record: &MessageRecord,
        extraction: &muninn_core::ports::AtomicExtractionBatch,
    ) -> MemoryResult<Option<Message>> {
        self.atomic_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self
            .fail_next_atomic
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(MemoryError::storage("simulated extraction write failure"));
        }
        let mut committed = self.committed_messages.lock().unwrap();
        if let Some(key) = record.operation_key.as_deref() {
            if let Some(existing) = committed
                .iter()
                .find(|r| r.operation_key.as_deref() == Some(key))
            {
                if existing.operation_payload != record.operation_payload {
                    return Err(MemoryError::invalid(
                        "operation_key conflicts with an existing payload or conversation scope",
                    ));
                }
                return Ok(Some(atomic_record_to_message(existing, conversation_id)?));
            }
        }
        committed.push(record.clone());
        self.committed_batches
            .lock()
            .unwrap()
            .push(extraction.clone());
        Ok(Some(atomic_record_to_message(record, conversation_id)?))
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
        _: &OwnerScope,
        _: Option<i64>,
    ) -> MemoryResult<Option<Conversation>> {
        Ok(Some(Conversation {
            id: CONVERSATION,
            ..Conversation::empty(session)
        }))
    }
    async fn get_conversation_by_id(
        &self,
        _: Uuid,
        _: &OwnerScope,
    ) -> MemoryResult<Option<Conversation>> {
        Ok(None)
    }
    async fn append_observation(&self, _: &Observation, _: &OwnerScope) -> MemoryResult<()> {
        unreachable!()
    }
    async fn list_observations(
        &self,
        _: Uuid,
        _: &OwnerScope,
        _: i64,
        _: i64,
    ) -> MemoryResult<Vec<Observation>> {
        unreachable!()
    }
    async fn list_conversations(&self, _: &OwnerScope, _: i64) -> MemoryResult<Vec<Conversation>> {
        unreachable!()
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
        unreachable!()
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
    async fn summary_entities(&self, _: &str, _: &OwnerScope) -> MemoryResult<Vec<String>> {
        unreachable!()
    }
}

struct CountingExtractor {
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl muninn_core::extraction::EntityExtractor for CountingExtractor {
    async fn extract(
        &self,
        text: &str,
        _: &muninn_core::extraction::ExtractOptions,
    ) -> MemoryResult<muninn_core::extraction::ExtractionResult> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(muninn_core::extraction::ExtractionResult {
            source_text: Some(text.to_owned()),
            entities: vec![
                muninn_core::extraction::ExtractedEntity {
                    name: "Alice".into(),
                    entity_type: "PERSON".into(),
                    subtype: None,
                    start_pos: Some(0),
                    end_pos: Some(5),
                    confidence: 0.95,
                    extractor: Some("test".into()),
                },
                muninn_core::extraction::ExtractedEntity {
                    name: "Neo4j".into(),
                    entity_type: "ORGANIZATION".into(),
                    subtype: None,
                    start_pos: Some(15),
                    end_pos: Some(20),
                    confidence: 0.95,
                    extractor: Some("test".into()),
                },
            ],
            relations: vec![muninn_core::extraction::ExtractedRelation {
                source: "Alice".into(),
                target: "Neo4j".into(),
                relation_type: "WORKS_AT".into(),
                confidence: 0.95,
            }],
            preferences: vec![],
        })
    }
}

#[tokio::test]
async fn atomic_ingestion_commits_single_txn_and_rolls_back_without_orphans() {
    use std::sync::atomic::Ordering;

    let short_term = Arc::new(AtomicShortTermStore::default());
    let shared = Arc::new(Store::default());
    let extractor = Arc::new(CountingExtractor {
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let client = MemoryClient::read_only(
        MemoryPorts {
            short_term: short_term.clone(),
            long_term: shared.clone(),
            reasoning: shared,
            embedder: None,
            cypher_query: None,
        },
        &MemoryOptions { multi_tenant: true },
    )
    .with_extractor(extractor.clone());

    let opts = crate::AddMessageOptions {
        operation_key: Some("op-atomic-1".into()),
        user_identifier: Some("owner-a".into()),
        extraction_mode: Some(muninn_core::extraction::ExtractionMode::Auto),
        generate_embedding: false,
        ..Default::default()
    };

    // 1. Mid-transaction extraction failure rolls back completely (zero orphan messages).
    short_term.fail_next_atomic.store(true, Ordering::SeqCst);
    let err = client
        .short_term()
        .add_message(
            "s-1",
            MessageRole::User,
            "Alice works at Neo4j",
            opts.clone(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, MemoryError::Storage(_)), "{err:?}");
    assert!(short_term.committed_messages.lock().unwrap().is_empty());
    assert_eq!(short_term.append_calls.load(Ordering::SeqCst), 0);

    // 2. Retry with same operation_key succeeds atomically in one call.
    let first = client
        .short_term()
        .add_message(
            "s-1",
            MessageRole::User,
            "Alice works at Neo4j",
            opts.clone(),
        )
        .await
        .expect("retry commits atomically");
    assert_eq!(short_term.atomic_calls.load(Ordering::SeqCst), 2);
    assert_eq!(short_term.append_calls.load(Ordering::SeqCst), 0);
    assert_eq!(short_term.committed_messages.lock().unwrap().len(), 1);
    {
        let batches = short_term.committed_batches.lock().unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].entities.len(), 2);
        assert_eq!(batches[0].relations.len(), 1);
    }

    // 3. Post-commit replay with identical operation_key returns existing message without re-extracting.
    let calls_before_replay = extractor.calls.load(Ordering::SeqCst);
    let replayed = client
        .short_term()
        .add_message("s-1", MessageRole::User, "Alice works at Neo4j", opts)
        .await
        .expect("idempotent replay succeeds");
    assert_eq!(replayed.id, first.id);
    assert_eq!(extractor.calls.load(Ordering::SeqCst), calls_before_replay);

    // 4. Reusing operation_key with different content fails closed.
    let conflict = client
        .short_term()
        .add_message(
            "s-1",
            MessageRole::User,
            "Different content",
            crate::AddMessageOptions {
                operation_key: Some("op-atomic-1".into()),
                user_identifier: Some("owner-a".into()),
                extraction_mode: Some(muninn_core::extraction::ExtractionMode::Auto),
                generate_embedding: false,
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(conflict, MemoryError::InvalidInput(_)),
        "{conflict:?}"
    );
}

#[tokio::test]
async fn memory_client_wires_buffered_adoption_spatial_and_eval_facades() {
    use crate::buffered::{BufferedWriter, BufferedWriterConfig};
    use muninn_core::config::WriteMode;
    use muninn_core::ports::{
        AdoptGraphOptions, AdoptionLabelReport, AdoptionReport, BoundingBox, GeoPoint,
        SchemaAdoptionPort, SessionLocation, SpatialLocationHit, SpatialStore, WriteExecutor,
    };

    struct NoopExec;
    #[async_trait]
    impl WriteExecutor for NoopExec {
        async fn execute_write(
            &self,
            _: &str,
            _: &serde_json::Map<String, serde_json::Value>,
        ) -> MemoryResult<()> {
            Ok(())
        }
    }

    struct StubAdoption;
    #[async_trait]
    impl SchemaAdoptionPort for StubAdoption {
        async fn adopt_existing_graph(
            &self,
            options: &AdoptGraphOptions,
        ) -> MemoryResult<AdoptionReport> {
            Ok(AdoptionReport {
                by_label: vec![AdoptionLabelReport {
                    label: "Person".into(),
                    entity_type: "PERSON".into(),
                    name_property: "name".into(),
                    migrated_count: 1,
                    already_adopted_count: 0,
                    skipped_count: 0,
                    conflict_count: 0,
                }],
                dry_run: options.dry_run,
            })
        }
    }

    struct StubSpatial;
    #[async_trait]
    impl SpatialStore for StubSpatial {
        async fn search_locations_near(
            &self,
            _: GeoPoint,
            _: f64,
            _: i64,
        ) -> MemoryResult<Vec<SpatialLocationHit>> {
            Ok(vec![])
        }
        async fn search_locations_in_bounding_box(
            &self,
            _: BoundingBox,
            _: i64,
        ) -> MemoryResult<Vec<SpatialLocationHit>> {
            Ok(vec![])
        }
        async fn get_location_coordinates(&self, _: Uuid) -> MemoryResult<Option<GeoPoint>> {
            Ok(Some(GeoPoint::new(40.4, -3.7).unwrap()))
        }
        async fn get_locations(
            &self,
            _: Option<&str>,
            _: i64,
        ) -> MemoryResult<Vec<SessionLocation>> {
            Ok(vec![])
        }
    }

    let shared = Arc::new(Store::default());
    let writer = Arc::new(
        BufferedWriter::new(
            Arc::new(NoopExec),
            BufferedWriterConfig {
                write_mode: WriteMode::Buffered,
                ..Default::default()
            },
        )
        .unwrap(),
    );

    // Read-only client rejects buffered writes and schema adoption fail-closed.
    let ro = MemoryClient::read_only(
        MemoryPorts {
            short_term: shared.clone(),
            long_term: shared.clone(),
            reasoning: shared.clone(),
            embedder: None,
            cypher_query: None,
        },
        &MemoryOptions::default(),
    );
    assert!(ro.buffered().is_none());
    assert!(ro.write_errors().is_empty());
    ro.flush().await.unwrap();
    ro.wait_for_pending().await.unwrap();
    ro.close().await.unwrap();
    assert!(ro
        .adopt_existing_graph(&AdoptGraphOptions::default())
        .await
        .is_err());
    assert!(ro
        .search_locations_near(GeoPoint::new(0.0, 0.0).unwrap(), 1.0, 5)
        .await
        .is_err());
    assert!(ro
        .search_locations_in_bounding_box(BoundingBox::new(0.0, 0.0, 1.0, 1.0).unwrap(), 5)
        .await
        .is_err());
    assert!(ro.get_location_coordinates(Uuid::new_v4()).await.is_err());
    assert!(ro.get_locations(None, 5).await.is_err());
    assert!(ro.with_buffered_writer(writer.clone()).is_err());

    let ro2 = MemoryClient::read_only(
        MemoryPorts {
            short_term: shared.clone(),
            long_term: shared.clone(),
            reasoning: shared.clone(),
            embedder: None,
            cypher_query: None,
        },
        &MemoryOptions::default(),
    );
    assert!(ro2.with_schema_adoption(Arc::new(StubAdoption)).is_err());

    // Write-enabled client wires buffered writer, schema adoption, spatial, and eval.
    let client = MemoryClient::new(
        MemoryPorts {
            short_term: shared.clone(),
            long_term: shared.clone(),
            reasoning: shared,
            embedder: None,
            cypher_query: None,
        },
        &MemoryOptions::default(),
    )
    .unwrap()
    .with_buffered_writer(writer)
    .unwrap()
    .with_schema_adoption(Arc::new(StubAdoption))
    .unwrap()
    .with_spatial_store(Arc::new(StubSpatial))
    .with_extractor(Arc::new(CountingExtractor {
        calls: std::sync::atomic::AtomicUsize::new(0),
    }));

    assert!(client.buffered().is_some());
    client
        .buffered()
        .unwrap()
        .submit("CREATE (:Ping)", serde_json::Map::new())
        .await
        .unwrap();
    client.flush().await.unwrap();
    client.wait_for_pending().await.unwrap();
    assert!(client.write_errors().is_empty());

    let adopt = client
        .adopt_existing_graph(&AdoptGraphOptions::default())
        .await
        .unwrap();
    assert_eq!(adopt.total_migrated(), 1);

    assert!(client
        .search_locations_near(GeoPoint::new(40.4, -3.7).unwrap(), 5.0, 10)
        .await
        .unwrap()
        .is_empty());
    assert!(client
        .search_locations_in_bounding_box(BoundingBox::new(35.0, -10.0, 44.0, 5.0).unwrap(), 10)
        .await
        .unwrap()
        .is_empty());
    assert!(client
        .get_location_coordinates(Uuid::new_v4())
        .await
        .unwrap()
        .is_some());
    assert!(client
        .get_locations(Some("s1"), 10)
        .await
        .unwrap()
        .is_empty());

    // Exercise EvalMemory extraction (entities + relations) and resolution dimensions.
    let eval_report = client
        .eval()
        .run(&crate::EvalRequest {
            retrieval: vec![],
            preferences: vec![],
            extraction: vec![
                crate::ExtractionCase {
                    text: "Alice works at Neo4j".into(),
                    expected_entities: vec![
                        crate::ExpectedEntity {
                            name: "Alice".into(),
                            entity_type: "PERSON".into(),
                        },
                        crate::ExpectedEntity {
                            name: "Neo4j".into(),
                            entity_type: "ORGANIZATION".into(),
                        },
                    ],
                    expected_relations: vec![],
                },
                crate::ExtractionCase {
                    text: "Alice works at Neo4j".into(),
                    expected_entities: vec![],
                    expected_relations: vec![crate::ExpectedRelation {
                        source: "Alice".into(),
                        relation_type: "WORKS_AT".into(),
                        target: "Neo4j".into(),
                    }],
                },
            ],
            resolution: vec![crate::ResolutionCase {
                predicted_clusters: std::collections::BTreeMap::from([
                    ("a".into(), "c1".into()),
                    ("b".into(), "c1".into()),
                ]),
                gold_clusters: std::collections::BTreeMap::from([
                    ("a".into(), "g1".into()),
                    ("b".into(), "g1".into()),
                ]),
            }],
        })
        .await
        .unwrap();
    assert_eq!(eval_report.extraction.as_ref().unwrap().cases, 2);
    assert!((eval_report.extraction.as_ref().unwrap().score - 1.0).abs() < 1e-9);
    assert!((eval_report.resolution.as_ref().unwrap().score - 1.0).abs() < 1e-9);
    assert!(eval_report.skipped.is_empty());

    client.close().await.unwrap();
}
