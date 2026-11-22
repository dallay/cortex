//! Typed, in-process operator access.

use muninn_core::extraction::ExtractionMode;
use muninn_core::model::{
    Conversation, ConversationTrace, Message, MessageRole, Preference, ReasoningTrace,
};
use muninn_core::{MemoryError, MemoryResult};
use uuid::Uuid;

use crate::{AddMessageOptions, AddPreferenceOptions, MemoryClient, StartTraceOptions};

/// Restricted operator entry points, separate from the legacy memory tiers.
pub struct LocalOperatorView<'a> {
    memory: &'a MemoryClient,
}

impl<'a> LocalOperatorView<'a> {
    pub(crate) const fn new(memory: &'a MemoryClient) -> Self {
        Self { memory }
    }

    pub const fn short_term(&self) -> LocalShortTerm<'a> {
        LocalShortTerm(self.memory)
    }

    pub const fn long_term(&self) -> LocalLongTerm<'a> {
        LocalLongTerm(self.memory)
    }

    pub const fn reasoning(&self) -> LocalReasoning<'a> {
        LocalReasoning(self.memory)
    }
}

/// Short-term entry point to the shared capability.
pub struct LocalShortTerm<'a>(&'a MemoryClient);

impl<'a> LocalShortTerm<'a> {
    pub fn client(&self) -> MemoryResult<LocalOperator<'a>> {
        self.0.graph()
    }
}

/// Long-term entry point to the shared capability.
pub struct LocalLongTerm<'a>(&'a MemoryClient);

impl<'a> LocalLongTerm<'a> {
    pub fn client(&self) -> MemoryResult<LocalOperator<'a>> {
        self.0.graph()
    }
}

/// Reasoning entry point to the shared capability.
pub struct LocalReasoning<'a>(&'a MemoryClient);

impl<'a> LocalReasoning<'a> {
    pub fn client(&self) -> MemoryResult<LocalOperator<'a>> {
        self.0.graph()
    }
}

/// The same typed owner-bound capability at all four entry points.
pub struct LocalOperator<'a> {
    memory: &'a MemoryClient,
    owner: &'a str,
    writes: bool,
}

impl<'a> LocalOperator<'a> {
    pub(crate) const fn new(memory: &'a MemoryClient, owner: &'a str, writes: bool) -> Self {
        Self {
            memory,
            owner,
            writes,
        }
    }

    fn require_writes(&self) -> MemoryResult<()> {
        if self.writes {
            Ok(())
        } else {
            Err(MemoryError::NotSupported(
                "local operator is read-only".into(),
            ))
        }
    }

    /// Read the owner's messages for this session.
    pub async fn get_conversation(
        &self,
        session_id: &str,
        limit: Option<i64>,
    ) -> MemoryResult<Conversation> {
        self.memory
            .short_term()
            .get_conversation(session_id, None, Some(self.owner), limit)
            .await
    }

    /// Append a message without embeddings or graph extraction.
    pub async fn add_message(
        &self,
        session_id: &str,
        role: MessageRole,
        content: &str,
    ) -> MemoryResult<Message> {
        self.require_writes()?;
        self.memory
            .short_term()
            .add_message(
                session_id,
                role,
                content,
                AddMessageOptions {
                    user_identifier: Some(self.owner.to_owned()),
                    generate_embedding: false,
                    extraction_mode: Some(ExtractionMode::Skip),
                    ..AddMessageOptions::default()
                },
            )
            .await
    }

    /// List preferences linked to the owner.
    pub async fn get_preferences(&self, active_only: bool) -> MemoryResult<Vec<Preference>> {
        self.memory
            .long_term()
            .get_preferences_for(self.owner, active_only)
            .await
    }

    /// Store a preference for the owner without embeddings or entity links.
    pub async fn add_preference(
        &self,
        category: &str,
        preference: &str,
    ) -> MemoryResult<Preference> {
        self.require_writes()?;
        self.memory
            .long_term()
            .add_preference(
                category,
                preference,
                AddPreferenceOptions {
                    user_identifier: Some(self.owner.to_owned()),
                    generate_embedding: false,
                    ..AddPreferenceOptions::default()
                },
            )
            .await
    }

    /// Read hosted steps and tool calls under an owner-scoped conversation.
    pub async fn get_conversation_trace(
        &self,
        conversation_id: Uuid,
    ) -> MemoryResult<ConversationTrace> {
        self.memory
            .reasoning()
            .get_conversation_trace(conversation_id, Some(self.owner))
            .await
    }

    /// Start a standalone trace for the owner without embeddings or message links.
    pub async fn start_trace(&self, session_id: &str, task: &str) -> MemoryResult<ReasoningTrace> {
        self.require_writes()?;
        self.memory
            .reasoning()
            .start_trace(
                session_id,
                task,
                StartTraceOptions {
                    user_identifier: Some(self.owner.to_owned()),
                    generate_embedding: false,
                    ..StartTraceOptions::default()
                },
            )
            .await
    }
}
