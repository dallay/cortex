//! Short-term memory use cases (`memory/short_term.py`).

use std::sync::Arc;

use muninn_core::model::{
    Conversation, ConversationSummary, Message, MessageRole, Metadata, NewMessage, Observation,
    SessionInfo,
};
use muninn_core::ports::{Embedder, MessageRecord, OwnerScope, SessionQuery, ShortTermStore};
use muninn_core::{MemoryError, MemoryResult};
use serde_json::Value;
use uuid::Uuid;

/// Default message cap applied by upstream `get_conversation` (`limit or 1000`).
const DEFAULT_CONVERSATION_LIMIT: i64 = 1000;

/// Options for [`ShortTermMemory::add_message`].
#[derive(Debug, Clone)]
pub struct AddMessageOptions {
    pub operation_key: Option<String>,
    pub conversation_id: Option<Uuid>,
    pub generate_embedding: bool,
    pub metadata: Metadata,
    pub user_identifier: Option<String>,
    pub extraction_mode: Option<muninn_core::extraction::ExtractionMode>,
    pub explicit_mentions: Vec<muninn_core::model::EntityRef>,
    pub extract_entities: Option<bool>,
    pub extract_relations: bool,
}

impl Default for AddMessageOptions {
    fn default() -> Self {
        Self {
            operation_key: None,
            conversation_id: None,
            generate_embedding: true,
            metadata: Metadata::new(),
            user_identifier: None,
            extraction_mode: None,
            explicit_mentions: Vec::new(),
            extract_entities: None,
            extract_relations: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SearchMessagesOptions {
    /// Caller-provided owner scope, not authentication or authorization.
    pub user_identifier: Option<String>,
    pub session_id: Option<String>,
    pub limit: i64,
    pub threshold: f64,
    pub metadata_filters: Metadata,
}

impl Default for SearchMessagesOptions {
    fn default() -> Self {
        Self {
            session_id: None,
            user_identifier: None,
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
    pub(crate) ingestion: Option<crate::extraction::IngestionExtraction>,
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
            ingestion: None,
        }
    }

    fn enforce_multi_tenant(&self, user_identifier: Option<&str>) -> MemoryResult<OwnerScope> {
        if user_identifier.is_some_and(|owner| owner.trim().is_empty()) {
            return Err(MemoryError::invalid("user_identifier must not be blank"));
        }
        if self.multi_tenant && user_identifier.is_none() {
            return Err(MemoryError::invalid(
                "multi_tenant is enabled but no user_identifier was supplied",
            ));
        }
        OwnerScope::from_option(user_identifier)
    }

    pub async fn add_message(
        &self,
        session_id: &str,
        role: MessageRole,
        content: &str,
        options: AddMessageOptions,
    ) -> MemoryResult<Message> {
        let scope = self.enforce_multi_tenant(options.user_identifier.as_deref())?;
        // The caller explicitly configured extraction. The pinned default
        // (`extraction_mode="auto"`, `extract_entities=True`) also extracts on
        // every message when a pipeline is configured, so the plan below runs
        // whenever ingestion is wired even without explicit arguments.
        let configured = options.extraction_mode.is_some()
            || options.extract_entities.is_some()
            || !options.explicit_mentions.is_empty();
        // Preserve the legacy payload byte-for-byte when extraction is unconfigured.
        let operation_payload = options.operation_key.as_ref().map(|_| {
            let mut payload = serde_json::json!({
                "role": role.as_str(), "content": content,
                "metadata": options.metadata, "generate_embedding": options.generate_embedding,
            });
            if configured {
                payload["extraction"] = serde_json::json!({"mode":options.extraction_mode,
                    "mentions":options.explicit_mentions,"entities":options.extract_entities,
                    "relations":options.extract_relations});
            }
            payload.to_string()
        });
        // Opt-in retries look up an existing conversation without creating one.
        let existing_conversation = if configured {
            self.store
                .get_conversation(session_id, options.conversation_id, &scope, Some(1))
                .await?
                .map(|c| c.id)
        } else {
            Some(
                self.store
                    .ensure_conversation(session_id, options.conversation_id, &scope)
                    .await?,
            )
        };
        if let (Some(operation_key), Some(payload), Some(conversation_id)) = (
            options.operation_key.as_deref(),
            operation_payload.as_deref(),
            existing_conversation,
        ) {
            if let Some(mut existing) = self
                .store
                .find_by_operation_key(conversation_id, &scope, operation_key)
                .await?
            {
                if existing.operation_payload.as_deref() != Some(payload) {
                    return Err(MemoryError::invalid(
                        "operation_key conflicts with an existing payload or conversation scope",
                    ));
                }
                // Match append_message's handling of metadata and return the
                // persisted message without calling the embedding provider.
                existing.message.metadata = options.metadata.clone();
                return Ok(existing.message);
            }
        }
        let plan = if configured || self.ingestion.is_some() {
            // Pinned default: `extraction_mode="auto"`. Without explicit args
            // the plan's Auto arm decides, and stays a no-op when the binary
            // configured no extraction stage.
            let ingestion = self
                .ingestion
                .as_ref()
                .ok_or_else(|| MemoryError::NotSupported("message extraction wiring".into()))?;
            Some(ingestion.prepare(content, &options).await?)
        } else {
            None
        };
        let embedding_identity = match (&self.embedder, options.generate_embedding) {
            (Some(embedder), true) => Some(checked_identity(embedder.as_ref())?),
            _ => None,
        };
        let embedding = match (&self.embedder, options.generate_embedding) {
            (Some(embedder), true) => Some(embedder.embed(content).await?),
            _ => None,
        };
        let conversation_id = match existing_conversation {
            Some(id) => id,
            None => {
                self.store
                    .ensure_conversation(session_id, options.conversation_id, &scope)
                    .await?
            }
        };
        let record = MessageRecord {
            operation_key: options.operation_key,
            operation_payload,
            id: Uuid::new_v4(),
            role: role.as_str().to_owned(),
            content: content.to_owned(),
            embedding,
            embedding_identity,
            metadata: options.metadata,
            timestamp: None,
        };
        let message = self
            .store
            .append_message(conversation_id, &scope, &record)
            .await?;
        if let (Some(ingestion), Some(plan)) = (&self.ingestion, plan) {
            ingestion.persist(message.id, content, plan).await?;
        }
        Ok(message)
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
        let scope = self.enforce_multi_tenant(user_identifier)?;
        if messages.is_empty() {
            return Ok(Vec::new());
        }
        let conversation_id = self
            .store
            .ensure_conversation(session_id, None, &scope)
            .await?;
        self.append_batches(
            conversation_id,
            &scope,
            messages,
            batch_size,
            generate_embeddings,
        )
        .await
    }

    /// Bulk add to an existing conversation (`bulk_add_messages`).
    pub async fn bulk_add_messages(
        &self,
        conversation_id: Uuid,
        messages: &[NewMessage],
        user_identifier: Option<&str>,
    ) -> MemoryResult<Vec<Message>> {
        let scope = self.enforce_multi_tenant(user_identifier)?;
        if self
            .store
            .get_conversation_by_id(conversation_id, &scope)
            .await?
            .is_none()
        {
            return Err(MemoryError::not_found(format!(
                "conversation {conversation_id}"
            )));
        }
        self.append_batches(conversation_id, &scope, messages, 100, true)
            .await
    }

    async fn append_batches(
        &self,
        conversation_id: Uuid,
        scope: &OwnerScope,
        messages: &[NewMessage],
        batch_size: usize,
        generate_embeddings: bool,
    ) -> MemoryResult<Vec<Message>> {
        let mut created = Vec::with_capacity(messages.len());
        for batch in messages.chunks(batch_size.max(1)) {
            let identity = match (&self.embedder, generate_embeddings) {
                (Some(embedder), true) => Some(checked_identity(embedder.as_ref())?),
                _ => None,
            };
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
                    let mut record = MessageRecord::from_new(message, embedding);
                    record.embedding_identity = identity.clone();
                    record
                })
                .collect();
            created.extend(
                self.store
                    .append_messages(conversation_id, scope, &records)
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
        user_identifier: Option<&str>,
        limit: Option<i64>,
    ) -> MemoryResult<Conversation> {
        let scope = self.enforce_multi_tenant(user_identifier)?;
        let limit = limit
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_CONVERSATION_LIMIT);
        Ok(self
            .store
            .get_conversation(session_id, conversation_id, &scope, Some(limit))
            .await?
            .unwrap_or_else(|| Conversation::empty(session_id)))
    }

    /// Explicitly create (or return) a session's conversation.
    pub async fn create_conversation(
        &self,
        session_id: &str,
        user_identifier: Option<&str>,
    ) -> MemoryResult<Conversation> {
        let scope = self.enforce_multi_tenant(user_identifier)?;
        self.store
            .ensure_conversation(session_id, None, &scope)
            .await?;
        self.get_conversation(session_id, None, user_identifier, None)
            .await
    }

    pub async fn get_conversation_by_id(
        &self,
        conversation_id: Uuid,
        user_identifier: Option<&str>,
    ) -> MemoryResult<Option<Conversation>> {
        let scope = self.enforce_multi_tenant(user_identifier)?;
        self.store
            .get_conversation_by_id(conversation_id, &scope)
            .await
    }

    pub async fn create_observation(
        &self,
        conversation_id: Uuid,
        content: &str,
        user_identifier: Option<&str>,
    ) -> MemoryResult<Observation> {
        let scope = self.enforce_multi_tenant(user_identifier)?;
        if content.trim().is_empty() {
            return Err(MemoryError::invalid(
                "observation content must not be blank",
            ));
        }
        let observation = Observation {
            id: Uuid::new_v4(),
            conversation_id,
            content: content.to_owned(),
            created_at: chrono::Utc::now(),
        };
        self.store.append_observation(&observation, &scope).await?;
        Ok(observation)
    }

    pub async fn get_observations(
        &self,
        conversation_id: Uuid,
        user_identifier: Option<&str>,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> MemoryResult<Vec<Observation>> {
        let scope = self.enforce_multi_tenant(user_identifier)?;
        let limit = limit.unwrap_or(100);
        let offset = offset.unwrap_or(0);
        if !(1..=500).contains(&limit) || offset < 0 {
            return Err(MemoryError::invalid(
                "observation pagination requires limit in 1..=500 and non-negative offset",
            ));
        }
        self.store
            .list_observations(conversation_id, &scope, limit, offset)
            .await
    }

    pub async fn list_conversations(
        &self,
        user_identifier: Option<&str>,
        limit: i64,
    ) -> MemoryResult<Vec<Conversation>> {
        let scope = self.enforce_multi_tenant(user_identifier)?;
        self.store.list_conversations(&scope, limit).await
    }

    pub async fn delete_conversation(
        &self,
        conversation_id: Uuid,
        user_identifier: Option<&str>,
    ) -> MemoryResult<()> {
        let scope = self.enforce_multi_tenant(user_identifier)?;
        self.store
            .delete_conversation(conversation_id, &scope)
            .await
    }

    /// Semantic search. Each hit carries its score in `metadata.similarity`,
    /// as upstream does. Without an embedder the result is empty.
    pub async fn search_messages(
        &self,
        query: &str,
        options: &SearchMessagesOptions,
    ) -> MemoryResult<Vec<Message>> {
        let scope = self.enforce_multi_tenant(options.user_identifier.as_deref())?;
        let Some(embedder) = &self.embedder else {
            return Ok(Vec::new());
        };
        if options.limit <= 0 {
            return Ok(Vec::new());
        }
        let identity = embedder.identity()?;
        if identity.dimensions() != embedder.dimensions() {
            return Err(MemoryError::invalid(
                "embedding identity dimensions mismatch",
            ));
        }
        let embedding = embedder.embed(query).await?;
        let hits = self
            .store
            .search_messages_with_identity(
                &embedding,
                &identity,
                options.session_id.as_deref(),
                &scope,
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

    pub async fn list_sessions(
        &self,
        query: &SessionQuery,
        user_identifier: Option<&str>,
    ) -> MemoryResult<Vec<SessionInfo>> {
        let scope = self.enforce_multi_tenant(user_identifier)?;
        self.store.list_sessions(query, &scope).await
    }

    pub async fn delete_message(
        &self,
        message_id: Uuid,
        user_identifier: Option<&str>,
        cascade: bool,
    ) -> MemoryResult<bool> {
        let scope = self.enforce_multi_tenant(user_identifier)?;
        self.store.delete_message(message_id, &scope, cascade).await
    }

    pub async fn clear_session(
        &self,
        session_id: &str,
        user_identifier: Option<&str>,
    ) -> MemoryResult<()> {
        let scope = self.enforce_multi_tenant(user_identifier)?;
        self.store.clear_session(session_id, &scope).await
    }

    /// Context block for prompts (`ShortTermMemory.get_context`).
    pub async fn get_context(
        &self,
        query: &str,
        session_id: Option<&str>,
        max_messages: usize,
    ) -> MemoryResult<String> {
        self.get_context_scoped(query, session_id, max_messages, None)
            .await
    }

    /// Build prompt context within a single owner namespace.
    pub async fn get_context_scoped(
        &self,
        query: &str,
        session_id: Option<&str>,
        max_messages: usize,
        user_identifier: Option<&str>,
    ) -> MemoryResult<String> {
        self.enforce_multi_tenant(user_identifier)?;
        let mut parts = Vec::new();
        if let Some(session_id) = session_id {
            let limit = i64::try_from(max_messages).unwrap_or(i64::MAX);
            let conversation = self
                .get_conversation(session_id, None, user_identifier, Some(limit))
                .await?;
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
                user_identifier: user_identifier.map(str::to_owned),
                session_id: session_id.map(str::to_owned),
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

    /// Generate a summary of a conversation (`get_conversation_summary`, Silver tier).
    ///
    /// When `summarizer` is `None` and no LLM provider is configured, falls back
    /// to a basic summary built from the first messages' content.
    pub async fn get_conversation_summary(
        &self,
        session_id: &str,
        user_identifier: Option<&str>,
        max_tokens: usize,
        include_entities: bool,
        summarizer: Option<impl FnOnce(String) -> String>,
    ) -> MemoryResult<ConversationSummary> {
        let scope = self.enforce_multi_tenant(user_identifier)?;
        let conversation = self
            .get_conversation(session_id, None, user_identifier, None)
            .await?;

        if conversation.messages.is_empty() {
            return Ok(ConversationSummary::empty(session_id));
        }

        // Build transcript
        let transcript = conversation
            .messages
            .iter()
            .map(|msg| format!("{}: {}", msg.role, msg.content))
            .collect::<Vec<_>>()
            .join("\n");

        // Time range
        let first_time = conversation.messages.first().map(|m| m.created_at);
        let last_time = conversation.messages.last().map(|m| m.created_at);
        let time_range = match (first_time, last_time) {
            (Some(f), Some(l)) => Some((f, l)),
            _ => None,
        };

        // Key entities
        let mut key_entities = Vec::new();
        if include_entities {
            key_entities = self.store.summary_entities(session_id, &scope).await?;
        }

        // Generate summary text
        let summary_text = if let Some(sum_fn) = summarizer {
            sum_fn(transcript)
        } else {
            self.build_basic_summary(&conversation.messages, max_tokens)
        };

        // Extract key topics from summary
        let key_topics = extract_key_topics(&summary_text);

        Ok(ConversationSummary {
            session_id: session_id.to_owned(),
            summary: summary_text,
            message_count: conversation.messages.len() as i64,
            time_range,
            key_entities,
            key_topics,
            generated_at: chrono::Utc::now(),
        })
    }

    /// Build a basic summary without an LLM — truncates the transcript.
    fn build_basic_summary(&self, messages: &[Message], max_tokens: usize) -> String {
        let max_chars = max_tokens.saturating_mul(4); // rough char-per-token estimate
        let mut parts = Vec::new();
        let mut total = 0usize;
        for msg in messages.iter().rev().take(10) {
            let line = format!("{}: {}", msg.role, msg.content);
            if total + line.len() > max_chars && !parts.is_empty() {
                break;
            }
            total += line.len();
            parts.push(line);
        }
        parts.reverse();
        if parts.is_empty() {
            "Conversation contains messages.".to_owned()
        } else {
            parts.join("\n")
        }
    }
}

/// Simple keyword extraction from summary text.
fn extract_key_topics(text: &str) -> Vec<String> {
    let mut words = text
        .split_whitespace()
        .filter(|w| w.len() > 3)
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>();
    words.sort();
    words.dedup();
    words.truncate(5);
    words
}

fn checked_identity(
    embedder: &dyn Embedder,
) -> MemoryResult<muninn_core::ports::EmbeddingIdentity> {
    let identity = embedder.identity()?;
    if identity.dimensions() != embedder.dimensions() {
        return Err(MemoryError::invalid(
            "embedding identity dimensions mismatch",
        ));
    }
    Ok(identity)
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    use async_trait::async_trait;
    use muninn_core::ports::EmbeddingIdentity;

    struct Unidentified;
    #[async_trait]
    impl Embedder for Unidentified {
        fn dimensions(&self) -> usize {
            3
        }
        async fn embed(&self, _: &str) -> MemoryResult<Vec<f64>> {
            panic!("identity must be checked before inference")
        }
    }

    struct Mismatched;
    #[async_trait]
    impl Embedder for Mismatched {
        fn dimensions(&self) -> usize {
            3
        }
        fn identity(&self) -> MemoryResult<EmbeddingIdentity> {
            EmbeddingIdentity::new("test:model:v1", 2)
        }
        async fn embed(&self, _: &str) -> MemoryResult<Vec<f64>> {
            panic!("identity must be checked before inference")
        }
    }

    #[test]
    fn unidentified_and_dimension_mismatched_providers_fail_before_inference() {
        assert!(checked_identity(&Unidentified).is_err());
        assert!(checked_identity(&Mismatched).is_err());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use chrono::{DateTime, Utc};
    use muninn_core::model::{Observation, SessionInfo};
    use muninn_core::ports::{KeyedMessage, OwnerScope, ScoredMessage, SessionQuery};
    use std::sync::Mutex;

    #[derive(Default)]
    struct SummaryStore {
        conversation: Mutex<Option<Conversation>>,
        entities: Vec<String>,
    }

    #[async_trait]
    impl ShortTermStore for SummaryStore {
        async fn ensure_conversation(
            &self,
            _: &str,
            _: Option<Uuid>,
            _: &OwnerScope,
        ) -> MemoryResult<Uuid> {
            panic!("unused in summary tests")
        }

        async fn find_by_operation_key(
            &self,
            _: Uuid,
            _: &OwnerScope,
            _: &str,
        ) -> MemoryResult<Option<KeyedMessage>> {
            panic!("unused in summary tests")
        }

        async fn append_message(
            &self,
            _: Uuid,
            _: &OwnerScope,
            _: &MessageRecord,
        ) -> MemoryResult<Message> {
            panic!("unused in summary tests")
        }

        async fn append_messages(
            &self,
            _: Uuid,
            _: &OwnerScope,
            _: &[MessageRecord],
        ) -> MemoryResult<Vec<Message>> {
            panic!("unused in summary tests")
        }

        async fn get_conversation(
            &self,
            _: &str,
            _: Option<Uuid>,
            _: &OwnerScope,
            _: Option<i64>,
        ) -> MemoryResult<Option<Conversation>> {
            Ok(self
                .conversation
                .lock()
                .expect("conversation lock is not poisoned")
                .clone())
        }

        async fn get_conversation_by_id(
            &self,
            _: Uuid,
            _: &OwnerScope,
        ) -> MemoryResult<Option<Conversation>> {
            panic!("unused in summary tests")
        }

        async fn append_observation(&self, _: &Observation, _: &OwnerScope) -> MemoryResult<()> {
            panic!("unused in summary tests")
        }

        async fn list_observations(
            &self,
            _: Uuid,
            _: &OwnerScope,
            _: i64,
            _: i64,
        ) -> MemoryResult<Vec<Observation>> {
            panic!("unused in summary tests")
        }

        async fn list_conversations(
            &self,
            _: &OwnerScope,
            _: i64,
        ) -> MemoryResult<Vec<Conversation>> {
            panic!("unused in summary tests")
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
            panic!("unused in summary tests")
        }

        async fn list_sessions(
            &self,
            _: &SessionQuery,
            _: &OwnerScope,
        ) -> MemoryResult<Vec<SessionInfo>> {
            panic!("unused in summary tests")
        }

        async fn delete_message(&self, _: Uuid, _: &OwnerScope, _: bool) -> MemoryResult<bool> {
            panic!("unused in summary tests")
        }

        async fn clear_session(&self, _: &str, _: &OwnerScope) -> MemoryResult<()> {
            panic!("unused in summary tests")
        }

        async fn delete_conversation(&self, _: Uuid, _: &OwnerScope) -> MemoryResult<()> {
            panic!("unused in summary tests")
        }

        async fn summary_entities(&self, _: &str, _: &OwnerScope) -> MemoryResult<Vec<String>> {
            Ok(self.entities.clone())
        }
    }

    fn timestamp(seconds: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(seconds, 0).expect("timestamp is representable")
    }

    fn message(role: MessageRole, content: &str, seconds: i64) -> Message {
        Message {
            id: Uuid::new_v4(),
            role,
            content: content.to_owned(),
            conversation_id: None,
            created_at: timestamp(seconds),
            embedding: None,
            metadata: Metadata::new(),
        }
    }

    fn memory(conversation: Option<Conversation>, entities: Vec<String>) -> ShortTermMemory {
        let store = SummaryStore {
            conversation: Mutex::new(conversation),
            entities,
        };
        ShortTermMemory::new(Arc::new(store), None, false)
    }

    #[test]
    fn extract_key_topics_filters_normalizes_sorts_and_limits() {
        assert_eq!(
            extract_key_topics("the alpha beta gamma delta epsilon zeta alpha ---- !!!a"),
            ["a", "alpha", "beta", "delta", "epsilon"]
        );
        assert_eq!(extract_key_topics("a and the"), Vec::<String>::new());
    }

    #[test]
    fn basic_summary_uses_the_latest_ten_messages_in_chronological_order() {
        let memory = memory(None, Vec::new());
        let messages = (0..12)
            .map(|index| message(MessageRole::User, &format!("message-{index}"), index))
            .collect::<Vec<_>>();

        let summary = memory.build_basic_summary(&messages, 100);
        assert!(summary.starts_with("user: message-2\n"));
        assert!(summary.ends_with("user: message-11"));
        assert_eq!(summary.lines().count(), 10);
    }

    #[test]
    fn basic_summary_observes_budget_and_empty_message_fallback() {
        let memory = memory(None, Vec::new());
        let messages = vec![
            message(MessageRole::User, "earlier", 1),
            message(MessageRole::Assistant, "latest", 2),
        ];

        assert_eq!(
            memory.build_basic_summary(&messages, 2),
            "assistant: latest"
        );
        assert_eq!(
            memory.build_basic_summary(&[], 0),
            "Conversation contains messages."
        );
        let oversized = vec![message(MessageRole::User, "longer than budget", 3)];
        assert_eq!(
            memory.build_basic_summary(&oversized, 0),
            "user: longer than budget"
        );
    }

    #[tokio::test]
    async fn conversation_summary_returns_empty_defaults_for_missing_messages() {
        let memory = memory(None, Vec::new());
        let summary = memory
            .get_conversation_summary("missing", None, 20, true, None::<fn(String) -> String>)
            .await
            .expect("summary succeeds");
        assert_eq!(summary.session_id, "missing");
        assert_eq!(summary.summary, "No messages in this conversation.");
        assert_eq!(summary.message_count, 0);
        assert_eq!(summary.time_range, None);
        assert!(summary.key_entities.is_empty());
        assert!(summary.key_topics.is_empty());
    }

    #[tokio::test]
    async fn conversation_summary_builds_transcript_and_optional_entities() {
        let first_time = timestamp(1_700_000_000);
        let last_time = timestamp(1_700_000_060);
        let conversation = Conversation {
            id: Uuid::new_v4(),
            session_id: "session".to_owned(),
            title: None,
            messages: vec![
                message(MessageRole::User, "Ada met Grace", 1_700_000_000),
                message(
                    MessageRole::Assistant,
                    "Grace designed compilers",
                    1_700_000_060,
                ),
            ],
            created_at: first_time,
            updated_at: Some(last_time),
            user_identifier: None,
        };
        let memory = memory(Some(conversation), vec!["Ada Lovelace".to_owned()]);
        let summary = memory
            .get_conversation_summary("session", None, 100, true, None::<fn(String) -> String>)
            .await
            .expect("summary succeeds");

        assert_eq!(
            summary.summary,
            "user: Ada met Grace\nassistant: Grace designed compilers"
        );
        assert_eq!(summary.message_count, 2);
        assert_eq!(summary.time_range, Some((first_time, last_time)));
        assert_eq!(summary.key_entities, ["Ada Lovelace"]);
        assert_eq!(
            summary.key_topics,
            ["Grace", "assistant", "compilers", "designed", "user"]
        );

        let summarized = memory
            .get_conversation_summary(
                "session",
                None,
                100,
                false,
                Some(|transcript| format!("custom: {transcript}")),
            )
            .await
            .expect("custom summary succeeds");
        assert_eq!(summarized.key_entities, Vec::<String>::new());
        assert!(summarized
            .summary
            .starts_with("custom: user: Ada met Grace"));
    }
}
