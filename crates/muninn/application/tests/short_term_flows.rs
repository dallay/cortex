use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

use async_trait::async_trait;
use chrono::Utc;
use muninn_application::{SearchMessagesOptions, ShortTermMemory};
use muninn_core::{
    model::{Conversation, Message, MessageRole, Metadata, NewMessage, SessionInfo},
    ports::{
        Embedder, EmbeddingIdentity, KeyedMessage, MessageRecord, OwnerScope, ScoredMessage,
        SessionQuery, ShortTermStore,
    },
    MemoryError, MemoryResult,
};
use uuid::Uuid;

struct Store {
    conversation_id: Uuid,
    ensure_calls: AtomicUsize,
    append_calls: AtomicUsize,
    records: Mutex<Vec<MessageRecord>>,
    conversation: Mutex<Option<Conversation>>,
    search_hits: Mutex<Vec<ScoredMessage>>,
    last_limit: Mutex<Option<i64>>,
    last_scope: Mutex<Option<OwnerScope>>,
    last_identity: Mutex<Option<String>>,
}

impl Default for Store {
    fn default() -> Self {
        Self {
            conversation_id: Uuid::new_v4(),
            ensure_calls: AtomicUsize::new(0),
            append_calls: AtomicUsize::new(0),
            records: Mutex::new(Vec::new()),
            conversation: Mutex::new(None),
            search_hits: Mutex::new(Vec::new()),
            last_limit: Mutex::new(None),
            last_scope: Mutex::new(None),
            last_identity: Mutex::new(None),
        }
    }
}

fn to_message(record: &MessageRecord, conversation_id: Uuid) -> MemoryResult<Message> {
    Ok(Message {
        id: record.id,
        role: record.role.parse()?,
        content: record.content.clone(),
        conversation_id: Some(conversation_id),
        created_at: record.timestamp.unwrap_or_else(Utc::now),
        embedding: record.embedding.clone(),
        metadata: record.metadata.clone(),
    })
}

#[async_trait]
impl ShortTermStore for Store {
    async fn ensure_conversation(
        &self,
        _: &str,
        _: Option<Uuid>,
        scope: &OwnerScope,
    ) -> MemoryResult<Uuid> {
        self.ensure_calls.fetch_add(1, Ordering::SeqCst);
        *self.last_scope.lock().unwrap() = Some(scope.clone());
        Ok(self.conversation_id)
    }

    async fn find_by_operation_key(
        &self,
        _: Uuid,
        _: &OwnerScope,
        _: &str,
    ) -> MemoryResult<Option<KeyedMessage>> {
        Ok(None)
    }

    async fn append_message(
        &self,
        conversation_id: Uuid,
        _: &OwnerScope,
        record: &MessageRecord,
    ) -> MemoryResult<Message> {
        self.append_calls.fetch_add(1, Ordering::SeqCst);
        self.records.lock().unwrap().push(record.clone());
        to_message(record, conversation_id)
    }

    async fn append_messages(
        &self,
        conversation_id: Uuid,
        _: &OwnerScope,
        records: &[MessageRecord],
    ) -> MemoryResult<Vec<Message>> {
        self.append_calls.fetch_add(1, Ordering::SeqCst);
        self.records.lock().unwrap().extend_from_slice(records);
        records
            .iter()
            .map(|record| to_message(record, conversation_id))
            .collect()
    }

    async fn get_conversation(
        &self,
        _: &str,
        _: Option<Uuid>,
        scope: &OwnerScope,
        limit: Option<i64>,
    ) -> MemoryResult<Option<Conversation>> {
        *self.last_limit.lock().unwrap() = limit;
        *self.last_scope.lock().unwrap() = Some(scope.clone());
        Ok(self.conversation.lock().unwrap().clone())
    }

    async fn get_conversation_by_id(
        &self,
        id: Uuid,
        scope: &OwnerScope,
    ) -> MemoryResult<Option<Conversation>> {
        *self.last_scope.lock().unwrap() = Some(scope.clone());
        Ok(self
            .conversation
            .lock()
            .unwrap()
            .clone()
            .filter(|conversation| conversation.id == id))
    }

    async fn list_conversations(
        &self,
        scope: &OwnerScope,
        limit: i64,
    ) -> MemoryResult<Vec<Conversation>> {
        *self.last_scope.lock().unwrap() = Some(scope.clone());
        *self.last_limit.lock().unwrap() = Some(limit);
        Ok(self
            .conversation
            .lock()
            .unwrap()
            .clone()
            .into_iter()
            .collect())
    }

    async fn search_messages_with_identity(
        &self,
        _: &[f64],
        identity: &EmbeddingIdentity,
        _: Option<&str>,
        scope: &OwnerScope,
        limit: i64,
        _: f64,
        _: &Metadata,
    ) -> MemoryResult<Vec<ScoredMessage>> {
        *self.last_scope.lock().unwrap() = Some(scope.clone());
        *self.last_identity.lock().unwrap() = Some(identity.key().to_owned());
        *self.last_limit.lock().unwrap() = Some(limit);
        Ok(self.search_hits.lock().unwrap().clone())
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
        Err(MemoryError::invalid("legacy search must not be used"))
    }

    async fn list_sessions(
        &self,
        _: &SessionQuery,
        scope: &OwnerScope,
    ) -> MemoryResult<Vec<SessionInfo>> {
        *self.last_scope.lock().unwrap() = Some(scope.clone());
        Ok(Vec::new())
    }

    async fn delete_message(&self, _: Uuid, scope: &OwnerScope, _: bool) -> MemoryResult<bool> {
        *self.last_scope.lock().unwrap() = Some(scope.clone());
        Ok(true)
    }

    async fn clear_session(&self, _: &str, scope: &OwnerScope) -> MemoryResult<()> {
        *self.last_scope.lock().unwrap() = Some(scope.clone());
        Ok(())
    }

    async fn delete_conversation(&self, _: Uuid, scope: &OwnerScope) -> MemoryResult<()> {
        *self.last_scope.lock().unwrap() = Some(scope.clone());
        Ok(())
    }
}

struct TestEmbedder {
    calls: AtomicUsize,
    batch_calls: AtomicUsize,
    identity: EmbeddingIdentity,
}

impl Default for TestEmbedder {
    fn default() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            batch_calls: AtomicUsize::new(0),
            identity: EmbeddingIdentity::new("test:short-term:v1", 2).unwrap(),
        }
    }
}

#[async_trait]
impl Embedder for TestEmbedder {
    fn dimensions(&self) -> usize {
        2
    }

    fn identity(&self) -> MemoryResult<EmbeddingIdentity> {
        Ok(self.identity.clone())
    }

    async fn embed(&self, _: &str) -> MemoryResult<Vec<f64>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(vec![1.0, 0.0])
    }

    async fn embed_batch(&self, texts: &[String]) -> MemoryResult<Vec<Vec<f64>>> {
        self.batch_calls.fetch_add(1, Ordering::SeqCst);
        Ok(texts.iter().map(|_| vec![1.0, 0.0]).collect())
    }
}

fn new_message(content: &str) -> NewMessage {
    NewMessage {
        role: MessageRole::User,
        content: content.to_owned(),
        metadata: Metadata::new(),
        timestamp: None,
    }
}

fn message(content: &str, conversation_id: Uuid) -> Message {
    Message {
        id: Uuid::new_v4(),
        role: MessageRole::Assistant,
        content: content.to_owned(),
        conversation_id: Some(conversation_id),
        created_at: Utc::now(),
        embedding: None,
        metadata: Metadata::new(),
    }
}

fn conversation(session_id: &str, messages: Vec<Message>) -> Conversation {
    Conversation {
        id: Uuid::new_v4(),
        session_id: session_id.to_owned(),
        title: None,
        messages,
        created_at: Utc::now(),
        updated_at: None,
        user_identifier: None,
    }
}

#[tokio::test]
async fn empty_batch_is_noop_but_still_enforces_owner_scope() {
    let store = Arc::new(Store::default());
    let memory = ShortTermMemory::new(store.clone(), None, false);
    assert!(memory
        .add_messages_batch("session", &[], 10, true, None)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(store.ensure_calls.load(Ordering::SeqCst), 0);

    let tenant = ShortTermMemory::new(store.clone(), None, true);
    assert!(tenant
        .add_messages_batch("session", &[], 10, true, None)
        .await
        .is_err());
    assert_eq!(store.ensure_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn batch_add_uses_identity_and_zero_batch_size_degrades_to_singletons() {
    let store = Arc::new(Store::default());
    let embedder = Arc::new(TestEmbedder::default());
    let memory = ShortTermMemory::new(store.clone(), Some(embedder.clone()), false);
    let created = memory
        .add_messages_batch(
            "session",
            &[new_message("one"), new_message("two")],
            0,
            true,
            Some("owner-a"),
        )
        .await
        .unwrap();
    assert_eq!(created.len(), 2);
    assert_eq!(store.append_calls.load(Ordering::SeqCst), 2);
    assert_eq!(embedder.batch_calls.load(Ordering::SeqCst), 2);
    {
        let records = store.records.lock().unwrap();
        assert_eq!(records.len(), 2);
        assert!(records.iter().all(|record| {
            record.embedding == Some(vec![1.0, 0.0])
                && record.embedding_identity.as_ref() == Some(&embedder.identity)
        }));
        drop(records);
    }
    assert_eq!(
        store.last_scope.lock().unwrap().as_ref().unwrap().as_str(),
        Some("owner-a")
    );
}

#[tokio::test]
async fn bulk_add_requires_existing_conversation_then_appends_with_embeddings() {
    let store = Arc::new(Store::default());
    let embedder = Arc::new(TestEmbedder::default());
    let memory = ShortTermMemory::new(store.clone(), Some(embedder.clone()), false);
    let missing = Uuid::new_v4();
    assert!(memory
        .bulk_add_messages(missing, &[new_message("missing")], None)
        .await
        .is_err());
    assert_eq!(store.append_calls.load(Ordering::SeqCst), 0);

    let existing = conversation("session", Vec::new());
    let id = existing.id;
    *store.conversation.lock().unwrap() = Some(existing);
    let result = memory
        .bulk_add_messages(id, &[new_message("saved")], Some("owner-b"))
        .await
        .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(store.append_calls.load(Ordering::SeqCst), 1);
    assert_eq!(embedder.batch_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn conversation_reads_normalize_limits_and_create_explicitly() {
    let store = Arc::new(Store::default());
    let memory = ShortTermMemory::new(store.clone(), None, false);
    let missing = memory
        .get_conversation("unknown", None, None, Some(0))
        .await
        .unwrap();
    assert_eq!(missing.session_id, "unknown");
    assert!(missing.messages.is_empty());
    assert_eq!(*store.last_limit.lock().unwrap(), Some(1000));

    let created = memory
        .create_conversation("new-session", Some("owner-c"))
        .await
        .unwrap();
    assert_eq!(created.session_id, "new-session");
    assert_eq!(store.ensure_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        store.last_scope.lock().unwrap().as_ref().unwrap().as_str(),
        Some("owner-c")
    );
    assert!(memory
        .get_conversation_by_id(Uuid::new_v4(), None)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn search_short_circuits_without_provider_and_attaches_score_with_identity() {
    let store = Arc::new(Store::default());
    let no_provider = ShortTermMemory::new(store.clone(), None, false);
    assert!(no_provider
        .search_messages("query", &SearchMessagesOptions::default())
        .await
        .unwrap()
        .is_empty());

    let embedder = Arc::new(TestEmbedder::default());
    let memory = ShortTermMemory::new(store.clone(), Some(embedder.clone()), false);
    let invalid_limit = SearchMessagesOptions {
        limit: 0,
        ..SearchMessagesOptions::default()
    };
    assert!(memory
        .search_messages("query", &invalid_limit)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(embedder.calls.load(Ordering::SeqCst), 0);

    let result = message("relevant", store.conversation_id);
    *store.search_hits.lock().unwrap() = vec![ScoredMessage {
        message: result,
        score: 0.875,
    }];
    let options = SearchMessagesOptions {
        user_identifier: Some("owner-c".into()),
        session_id: Some("session-c".into()),
        limit: 4,
        ..SearchMessagesOptions::default()
    };
    let hits = memory.search_messages("query", &options).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].metadata["similarity"], 0.875);
    assert_eq!(
        *store.last_identity.lock().unwrap(),
        Some(embedder.identity.key().to_owned())
    );
    assert_eq!(*store.last_limit.lock().unwrap(), Some(4));
    assert_eq!(embedder.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn context_combines_recent_conversation_and_scoped_search_results() {
    let store = Arc::new(Store::default());
    let embedder = Arc::new(TestEmbedder::default());
    let memory = ShortTermMemory::new(store.clone(), Some(embedder), false);
    *store.conversation.lock().unwrap() = Some(conversation(
        "session-d",
        vec![message("recent", store.conversation_id)],
    ));
    *store.search_hits.lock().unwrap() = vec![ScoredMessage {
        message: message("past", store.conversation_id),
        score: 0.75,
    }];
    let context = memory
        .get_context_scoped("query", Some("session-d"), 5, Some("owner-d"))
        .await
        .unwrap();
    assert!(context.contains("### Recent Conversation"));
    assert!(context.contains("### Relevant Past Messages"));
    assert!(context.contains("relevance: 0.75"));
    assert_eq!(
        store.last_scope.lock().unwrap().as_ref().unwrap().as_str(),
        Some("owner-d")
    );

    let empty = memory.get_context("query", None, 5).await.unwrap();
    assert!(empty.contains("Relevant Past Messages"));
}

#[tokio::test]
async fn listing_and_deletion_operations_preserve_the_owner_scope() {
    let store = Arc::new(Store::default());
    let memory = ShortTermMemory::new(store.clone(), None, false);
    let conversations = memory.list_conversations(Some("owner-e"), 7).await.unwrap();
    assert!(conversations.is_empty());
    assert_eq!(*store.last_limit.lock().unwrap(), Some(7));
    assert!(memory
        .list_sessions(&SessionQuery::default(), Some("owner-e"))
        .await
        .unwrap()
        .is_empty());
    assert!(memory
        .delete_message(Uuid::new_v4(), Some("owner-e"), false)
        .await
        .unwrap());
    memory
        .clear_session("session", Some("owner-e"))
        .await
        .unwrap();
    memory
        .delete_conversation(Uuid::new_v4(), Some("owner-e"))
        .await
        .unwrap();
    assert_eq!(
        store.last_scope.lock().unwrap().as_ref().unwrap().as_str(),
        Some("owner-e")
    );
}
