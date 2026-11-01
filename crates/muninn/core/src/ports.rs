//! Ports implemented by infrastructure adapters.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::error::MemoryResult;
use crate::model::{Conversation, Message, Metadata, NewMessage, SessionInfo};

/// Text embedding provider (`embeddings.base.Embedder`).
#[async_trait]
pub trait Embedder: Send + Sync {
    /// Vector dimensions produced by this embedder.
    fn dimensions(&self) -> usize;

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
    pub id: Uuid,
    pub role: String,
    pub content: String,
    pub embedding: Option<Vec<f64>>,
    pub metadata: Metadata,
    pub timestamp: Option<DateTime<Utc>>,
}

impl MessageRecord {
    pub fn from_new(message: &NewMessage, embedding: Option<Vec<f64>>) -> Self {
        Self {
            id: Uuid::new_v4(),
            role: message.role.as_str().to_owned(),
            content: message.content.clone(),
            embedding,
            metadata: message.metadata.clone(),
            timestamp: message.timestamp,
        }
    }
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

/// A message hit returned by vector search.
#[derive(Debug, Clone)]
pub struct ScoredMessage {
    pub message: Message,
    pub score: f64,
}

/// Persistence port for short-term (conversation) memory.
#[async_trait]
pub trait ShortTermStore: Send + Sync {
    /// Return the conversation id for `session_id`, creating it when absent.
    async fn ensure_conversation(
        &self,
        session_id: &str,
        conversation_id: Option<Uuid>,
        user_identifier: Option<&str>,
    ) -> MemoryResult<Uuid>;

    /// Append one message to the tail of a conversation's chain.
    async fn append_message(
        &self,
        conversation_id: Uuid,
        message: &MessageRecord,
    ) -> MemoryResult<Message>;

    /// Append messages in input order in one transaction.
    async fn append_messages(
        &self,
        conversation_id: Uuid,
        messages: &[MessageRecord],
    ) -> MemoryResult<Vec<Message>>;

    async fn get_conversation(
        &self,
        session_id: &str,
        conversation_id: Option<Uuid>,
        limit: Option<i64>,
    ) -> MemoryResult<Option<Conversation>>;

    async fn get_conversation_by_id(
        &self,
        conversation_id: Uuid,
    ) -> MemoryResult<Option<Conversation>>;

    async fn list_conversations(
        &self,
        user_identifier: Option<&str>,
        limit: i64,
    ) -> MemoryResult<Vec<Conversation>>;

    async fn search_messages(
        &self,
        embedding: &[f64],
        session_id: Option<&str>,
        limit: i64,
        threshold: f64,
        metadata_filters: &Metadata,
    ) -> MemoryResult<Vec<ScoredMessage>>;

    async fn list_sessions(&self, query: &SessionQuery) -> MemoryResult<Vec<SessionInfo>>;

    /// Delete a message, repairing the chain. Returns whether it existed.
    async fn delete_message(&self, message_id: Uuid, cascade: bool) -> MemoryResult<bool>;

    async fn clear_session(&self, session_id: &str) -> MemoryResult<()>;

    async fn delete_conversation(&self, conversation_id: Uuid) -> MemoryResult<()>;
}

/// Destructive maintenance operations, reserved for disposable test graphs.
#[async_trait]
pub trait GraphMaintenance: Send + Sync {
    /// Delete every node and relationship.
    async fn clear_all_data(&self) -> MemoryResult<()>;
}
