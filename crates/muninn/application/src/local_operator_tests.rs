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
