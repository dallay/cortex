//! Integration coverage for the conversation summary use case.
//!
//! The summary helpers stay private, so every assertion below goes through
//! the public `get_conversation_summary` entry point and observes the
//! returned `ConversationSummary`.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use muninn_application::ShortTermMemory;
use muninn_core::model::{Conversation, Message, MessageRole, Metadata, Observation, SessionInfo};
use muninn_core::ports::{KeyedMessage, MessageRecord, OwnerScope, ScoredMessage, SessionQuery};
use muninn_core::MemoryResult;
use uuid::Uuid;

/// A store that only serves one prepared conversation and its entities.
#[derive(Default)]
struct SummaryStore {
    conversation: Mutex<Option<Conversation>>,
    entities: Vec<String>,
}

#[async_trait]
impl muninn_core::ports::ShortTermStore for SummaryStore {
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

    async fn list_conversations(&self, _: &OwnerScope, _: i64) -> MemoryResult<Vec<Conversation>> {
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

fn conversation(session_id: &str, messages: Vec<Message>) -> Conversation {
    let created_at = messages
        .first()
        .map_or_else(Utc::now, |message| message.created_at);
    let updated_at = messages.last().map(|message| message.created_at);
    Conversation {
        id: Uuid::new_v4(),
        session_id: session_id.to_owned(),
        title: None,
        messages,
        created_at,
        updated_at,
        user_identifier: None,
    }
}

fn memory(conversation: Option<Conversation>, entities: Vec<String>) -> ShortTermMemory {
    let store = SummaryStore {
        conversation: Mutex::new(conversation),
        entities,
    };
    ShortTermMemory::new(Arc::new(store), None, false)
}

#[tokio::test]
async fn conversation_summary_is_empty_for_a_conversation_without_messages() {
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
async fn conversation_summary_lists_the_latest_ten_messages_in_chronological_order() {
    let messages = (0..12)
        .map(|index| message(MessageRole::User, &format!("message-{index}"), index))
        .collect::<Vec<_>>();
    let memory = memory(Some(conversation("session", messages)), Vec::new());

    let summary = memory
        .get_conversation_summary("session", None, 100, false, None::<fn(String) -> String>)
        .await
        .expect("summary succeeds");

    // The full sequence is asserted, so a swapped or reordered line fails.
    let expected = (2..12)
        .map(|index| format!("user: message-{index}"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(summary.summary, expected);
    assert_eq!(summary.summary.lines().count(), 10);
    assert_eq!(summary.message_count, 12);
}

#[tokio::test]
async fn conversation_summary_keeps_the_newest_messages_within_the_token_budget() {
    let memory = memory(
        Some(conversation(
            "session",
            vec![
                message(MessageRole::User, "earlier", 1),
                message(MessageRole::Assistant, "latest", 2),
            ],
        )),
        Vec::new(),
    );

    let summary = memory
        .get_conversation_summary("session", None, 2, false, None::<fn(String) -> String>)
        .await
        .expect("summary succeeds");
    assert_eq!(summary.summary, "assistant: latest");
}

#[tokio::test]
async fn conversation_summary_keeps_a_single_message_that_exceeds_the_token_budget() {
    let memory = memory(
        Some(conversation(
            "session",
            vec![message(MessageRole::User, "longer than budget", 3)],
        )),
        Vec::new(),
    );

    let summary = memory
        .get_conversation_summary("session", None, 0, false, None::<fn(String) -> String>)
        .await
        .expect("summary succeeds");
    assert_eq!(summary.summary, "user: longer than budget");
}

#[tokio::test]
async fn conversation_summary_reports_the_transcript_the_message_count_and_the_time_range() {
    let first_time = timestamp(1_700_000_000);
    let last_time = timestamp(1_700_000_060);
    let memory = memory(
        Some(conversation(
            "session",
            vec![
                message(MessageRole::User, "Ada met Grace", 1_700_000_000),
                message(
                    MessageRole::Assistant,
                    "Grace designed compilers",
                    1_700_000_060,
                ),
            ],
        )),
        vec!["Ada Lovelace".to_owned()],
    );

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
}

#[tokio::test]
async fn conversation_summary_omits_key_entities_unless_they_are_requested() {
    let memory = memory(
        Some(conversation(
            "session",
            vec![message(MessageRole::User, "Ada met Grace", 1_700_000_000)],
        )),
        vec!["Ada Lovelace".to_owned()],
    );

    let summary = memory
        .get_conversation_summary("session", None, 100, false, None::<fn(String) -> String>)
        .await
        .expect("summary succeeds");
    assert!(summary.key_entities.is_empty());
}

#[tokio::test]
async fn conversation_summary_uses_the_summarizer_and_derives_the_key_topics() {
    let memory = memory(
        Some(conversation(
            "session",
            vec![
                message(MessageRole::User, "Ada met Grace", 1_700_000_000),
                message(
                    MessageRole::Assistant,
                    "Grace designed compilers",
                    1_700_000_060,
                ),
            ],
        )),
        Vec::new(),
    );

    let transcript = Arc::new(Mutex::new(String::new()));
    let captured = Arc::clone(&transcript);
    let summary = memory
        .get_conversation_summary(
            "session",
            None,
            100,
            false,
            Some(move |transcript: String| {
                *captured.lock().expect("transcript lock is not poisoned") = transcript;
                "the alpha beta gamma delta epsilon zeta alpha ---- !!!a".to_owned()
            }),
        )
        .await
        .expect("summary succeeds");

    // The summarizer receives the whole transcript, not a truncated one.
    assert_eq!(
        *transcript.lock().expect("transcript lock is not poisoned"),
        "user: Ada met Grace\nassistant: Grace designed compilers"
    );
    assert_eq!(
        summary.summary,
        "the alpha beta gamma delta epsilon zeta alpha ---- !!!a"
    );
    // Short words, punctuation-only tokens and duplicates are dropped, and the
    // remaining topics are sorted and capped at five entries.
    assert_eq!(
        summary.key_topics,
        ["a", "alpha", "beta", "delta", "epsilon"]
    );
}
