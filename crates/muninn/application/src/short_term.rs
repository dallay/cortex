//! Short-term memory use cases (`memory/short_term.py`).

use std::sync::Arc;

use muninn_core::model::{Conversation, Message, MessageRole, Metadata, NewMessage, SessionInfo};
use muninn_core::ports::{Embedder, MessageRecord, SessionQuery, ShortTermStore};
use muninn_core::{MemoryError, MemoryResult};
use serde_json::Value;
use uuid::Uuid;

/// Default message cap applied by upstream `get_conversation` (`limit or 1000`).
const DEFAULT_CONVERSATION_LIMIT: i64 = 1000;

/// Options for [`ShortTermMemory::add_message`].
#[derive(Debug, Clone)]
pub struct AddMessageOptions {
    pub conversation_id: Option<Uuid>,
    pub generate_embedding: bool,
    pub metadata: Metadata,
    pub user_identifier: Option<String>,
}

impl Default for AddMessageOptions {
    fn default() -> Self {
        Self {
            conversation_id: None,
            generate_embedding: true,
            metadata: Metadata::new(),
            user_identifier: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SearchMessagesOptions {
    pub session_id: Option<String>,
    pub limit: i64,
    pub threshold: f64,
    pub metadata_filters: Metadata,
}

impl Default for SearchMessagesOptions {
    fn default() -> Self {
        Self {
            session_id: None,
            limit: 10,
            threshold: 0.7,
            metadata_filters: Metadata::new(),
        }
    }
}

pub struct ShortTermMemory {
    store: Arc<dyn ShortTermStore>,
    embedder: Option<Arc<dyn Embedder>>,
    multi_tenant: bool,
}

impl ShortTermMemory {
    pub fn new(
        store: Arc<dyn ShortTermStore>,
        embedder: Option<Arc<dyn Embedder>>,
        multi_tenant: bool,
    ) -> Self {
        Self {
            store,
            embedder,
            multi_tenant,
        }
    }

    fn enforce_multi_tenant(&self, user_identifier: Option<&str>) -> MemoryResult<()> {
        if self.multi_tenant && user_identifier.is_none() {
            return Err(MemoryError::invalid(
                "multi_tenant is enabled but no user_identifier was supplied",
            ));
        }
        Ok(())
    }

    pub async fn add_message(
        &self,
        session_id: &str,
        role: MessageRole,
        content: &str,
        options: AddMessageOptions,
    ) -> MemoryResult<Message> {
        self.enforce_multi_tenant(options.user_identifier.as_deref())?;
        let conversation_id = self
            .store
            .ensure_conversation(
                session_id,
                options.conversation_id,
                options.user_identifier.as_deref(),
            )
            .await?;
        let embedding = match (&self.embedder, options.generate_embedding) {
            (Some(embedder), true) => Some(embedder.embed(content).await?),
            _ => None,
        };
        let record = MessageRecord {
            id: Uuid::new_v4(),
            role: role.as_str().to_owned(),
            content: content.to_owned(),
            embedding,
            metadata: options.metadata,
            timestamp: None,
        };
        self.store.append_message(conversation_id, &record).await
    }

    /// `add_messages_batch`: one transaction per `batch_size` messages.
    pub async fn add_messages_batch(
        &self,
        session_id: &str,
        messages: &[NewMessage],
        batch_size: usize,
        generate_embeddings: bool,
        user_identifier: Option<&str>,
    ) -> MemoryResult<Vec<Message>> {
        if messages.is_empty() {
            return Ok(Vec::new());
        }
        self.enforce_multi_tenant(user_identifier)?;
        let conversation_id = self
            .store
            .ensure_conversation(session_id, None, user_identifier)
            .await?;
        self.append_batches(conversation_id, messages, batch_size, generate_embeddings)
            .await
    }

    /// Bulk add to an existing conversation (`bulk_add_messages`).
    pub async fn bulk_add_messages(
        &self,
        conversation_id: Uuid,
        messages: &[NewMessage],
    ) -> MemoryResult<Vec<Message>> {
        if self
            .store
            .get_conversation_by_id(conversation_id)
            .await?
            .is_none()
        {
            return Err(MemoryError::not_found(format!(
                "conversation {conversation_id}"
            )));
        }
        self.append_batches(conversation_id, messages, 100, true)
            .await
    }

    async fn append_batches(
        &self,
        conversation_id: Uuid,
        messages: &[NewMessage],
        batch_size: usize,
        generate_embeddings: bool,
    ) -> MemoryResult<Vec<Message>> {
        let mut created = Vec::with_capacity(messages.len());
        for batch in messages.chunks(batch_size.max(1)) {
            let embeddings = match (&self.embedder, generate_embeddings) {
                (Some(embedder), true) => {
                    let texts: Vec<String> = batch.iter().map(|m| m.content.clone()).collect();
                    Some(embedder.embed_batch(&texts).await?)
                }
                _ => None,
            };
            let records: Vec<MessageRecord> = batch
                .iter()
                .enumerate()
                .map(|(index, message)| {
                    let embedding = embeddings.as_ref().and_then(|e| e.get(index).cloned());
                    MessageRecord::from_new(message, embedding)
                })
                .collect();
            created.extend(
                self.store
                    .append_messages(conversation_id, &records)
                    .await?,
            );
        }
        Ok(created)
    }

    /// Return the session's latest conversation, or an empty one.
    pub async fn get_conversation(
        &self,
        session_id: &str,
        conversation_id: Option<Uuid>,
        limit: Option<i64>,
    ) -> MemoryResult<Conversation> {
        let limit = limit
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_CONVERSATION_LIMIT);
        Ok(self
            .store
            .get_conversation(session_id, conversation_id, Some(limit))
            .await?
            .unwrap_or_else(|| Conversation::empty(session_id)))
    }

    /// Explicitly create (or return) a session's conversation.
    pub async fn create_conversation(
        &self,
        session_id: &str,
        user_identifier: Option<&str>,
    ) -> MemoryResult<Conversation> {
        self.store
            .ensure_conversation(session_id, None, user_identifier)
            .await?;
        self.get_conversation(session_id, None, None).await
    }

    pub async fn get_conversation_by_id(
        &self,
        conversation_id: Uuid,
    ) -> MemoryResult<Option<Conversation>> {
        self.store.get_conversation_by_id(conversation_id).await
    }

    pub async fn list_conversations(
        &self,
        user_identifier: Option<&str>,
        limit: i64,
    ) -> MemoryResult<Vec<Conversation>> {
        self.store.list_conversations(user_identifier, limit).await
    }

    pub async fn delete_conversation(&self, conversation_id: Uuid) -> MemoryResult<()> {
        self.store.delete_conversation(conversation_id).await
    }

    /// Semantic search. Each hit carries its score in `metadata.similarity`,
    /// as upstream does. Without an embedder the result is empty.
    pub async fn search_messages(
        &self,
        query: &str,
        options: &SearchMessagesOptions,
    ) -> MemoryResult<Vec<Message>> {
        let Some(embedder) = &self.embedder else {
            return Ok(Vec::new());
        };
        if options.limit <= 0 {
            return Ok(Vec::new());
        }
        let embedding = embedder.embed(query).await?;
        let hits = self
            .store
            .search_messages(
                &embedding,
                options.session_id.as_deref(),
                options.limit,
                options.threshold,
                &options.metadata_filters,
            )
            .await?;
        Ok(hits
            .into_iter()
            .map(|hit| {
                let mut message = hit.message;
                message
                    .metadata
                    .insert("similarity".to_owned(), Value::from(hit.score));
                message
            })
            .collect())
    }

    pub async fn list_sessions(&self, query: &SessionQuery) -> MemoryResult<Vec<SessionInfo>> {
        self.store.list_sessions(query).await
    }

    pub async fn delete_message(&self, message_id: Uuid, cascade: bool) -> MemoryResult<bool> {
        self.store.delete_message(message_id, cascade).await
    }

    pub async fn clear_session(&self, session_id: &str) -> MemoryResult<()> {
        self.store.clear_session(session_id).await
    }

    /// Context block for prompts (`ShortTermMemory.get_context`).
    pub async fn get_context(
        &self,
        query: &str,
        session_id: Option<&str>,
        max_messages: usize,
    ) -> MemoryResult<String> {
        let mut parts = Vec::new();
        if let Some(session_id) = session_id {
            let limit = i64::try_from(max_messages).unwrap_or(i64::MAX);
            let conversation = self.get_conversation(session_id, None, Some(limit)).await?;
            if !conversation.messages.is_empty() {
                parts.push("### Recent Conversation".to_owned());
                let skip = conversation.messages.len().saturating_sub(max_messages);
                for message in conversation.messages.iter().skip(skip) {
                    parts.push(format!("**{}**: {}", message.role, message.content));
                }
            }
        }
        if self.embedder.is_some() {
            let options = SearchMessagesOptions {
                limit: 5,
                ..SearchMessagesOptions::default()
            };
            let relevant = self.search_messages(query, &options).await?;
            if !relevant.is_empty() {
                parts.push("\n### Relevant Past Messages".to_owned());
                for message in relevant {
                    let score = message
                        .metadata
                        .get("similarity")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0);
                    parts.push(format!(
                        "- [{}] {} (relevance: {score:.2})",
                        message.role, message.content
                    ));
                }
            }
        }
        Ok(parts.join("\n"))
    }
}
