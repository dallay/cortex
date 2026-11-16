//! Base memory types and protocols.
//!
//! This module provides the Rust equivalent of `neo4j_agent_memory.core.memory`:
//! the foundational types that every memory tier shares — the base entry model,
//! the generic store protocol, and shared configuration.
//!
//! It also re-exports [`ToolCallStatus`] from [`crate::model`] for convenience,
//! matching the Python location where the enum lives.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::error::MemoryResult;
use crate::model::Metadata;

// Re-export from model.rs — matches Python's location in core.memory.
pub use crate::model::ToolCallStatus;

/// Base model for all memory entries (`core.memory.MemoryEntry`).
///
/// Every memory tier (short-term, long-term, reasoning) produces entries
/// that carry these common fields: identity, timestamps, optional embedding,
/// and free-form metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryEntry {
    /// Unique identifier, auto-generated on creation.
    pub id: Uuid,
    /// Creation timestamp in UTC.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp; `None` when the entry has never been modified.
    pub updated_at: Option<DateTime<Utc>>,
    /// Optional vector embedding for similarity search.
    pub embedding: Option<Vec<f64>>,
    /// Free-form JSON object attached to the entry.
    pub metadata: Metadata,
}

impl MemoryEntry {
    /// Create a new entry with a generated UUID and current UTC time.
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            created_at: Utc::now(),
            updated_at: None,
            embedding: None,
            metadata: Metadata::new(),
        }
    }

    /// Create a new entry with explicit metadata.
    pub fn with_metadata(metadata: Metadata) -> Self {
        Self {
            metadata,
            ..Self::new()
        }
    }

    /// Attach an embedding vector to this entry.
    pub fn with_embedding(mut self, embedding: Vec<f64>) -> Self {
        self.embedding = Some(embedding);
        self
    }

    /// Mark this entry as updated at the current time.
    pub fn touch(&mut self) {
        self.updated_at = Some(Utc::now());
    }
}

impl Default for MemoryEntry {
    fn default() -> Self {
        Self::new()
    }
}

/// Generic memory store protocol (`core.memory.MemoryStore`).
///
/// Mirrors the Python `MemoryStore` Protocol. Any memory tier that supports
/// basic CRUD + vector search can implement this trait. It is the lowest
/// common denominator across short-term, long-term, and reasoning memory.
///
/// In practice, Muninn uses the more specific traits in [`crate::ports`]
/// (`ShortTermStore`, `LongTermStore`, `ReasoningStore`). This trait exists
/// for callers that want a tier-agnostic interface.
#[async_trait::async_trait]
pub trait MemoryStore: Send + Sync {
    /// The concrete entry type this store handles.
    type Entry: MemoryEntryLike + Send + Sync;

    /// Persist one entry, optionally scoped to a session.
    async fn store(
        &self,
        entry: Self::Entry,
        session_id: Option<&str>,
    ) -> MemoryResult<Self::Entry>;

    /// Retrieve one entry by its UUID.
    async fn retrieve(&self, entry_id: Uuid) -> MemoryResult<Option<Self::Entry>>;

    /// Search by semantic similarity.
    async fn search(
        &self,
        query: &str,
        limit: i64,
        threshold: f64,
        filters: &Metadata,
    ) -> MemoryResult<Vec<Self::Entry>>;

    /// Delete one entry by its UUID. Returns `true` when the entry existed.
    async fn delete(&self, entry_id: Uuid) -> MemoryResult<bool>;

    /// List entries with optional filters.
    async fn list(
        &self,
        session_id: Option<&str>,
        since: Option<DateTime<Utc>>,
        until: Option<DateTime<Utc>>,
        limit: i64,
    ) -> MemoryResult<Vec<Self::Entry>>;
}

/// Minimal behaviour every [`MemoryStore::Entry`] must provide.
///
/// This is a sealed-like trait that the concrete entry types implement so
/// [`MemoryStore`] can remain generic. Callers should not need to reference
/// it directly.
pub trait MemoryEntryLike: Send + Sync {
    /// The entry's stable identifier.
    fn entry_id(&self) -> Uuid;

    /// Optional embedding vector for similarity search.
    fn embedding(&self) -> Option<&[f64]>;
}

/// Shared configuration for all memory tiers (`core.memory` config surface).
///
/// Controls behaviour that applies across short-term, long-term, and reasoning
/// memory: embedding generation, entity extraction, and enrichment.
#[derive(Debug, Clone)]
pub struct MemoryConfig {
    /// Whether to generate vector embeddings by default.
    pub generate_embedding: bool,
    /// Whether to extract entities automatically.
    pub extract_entities: bool,
    /// Whether to extract relationships automatically.
    pub extract_relations: bool,
    /// Default similarity threshold for vector search.
    pub search_threshold: f64,
    /// Default result limit for vector search.
    pub search_limit: i64,
    /// Whether to geocode locations.
    pub geocode: bool,
    /// Whether to enrich entities with external data.
    pub enrich: bool,
    /// Whether to deduplicate on write.
    pub deduplicate: bool,
}

impl MemoryConfig {
    /// Defaults matching the Python reference implementation.
    pub fn default_agent() -> Self {
        Self {
            generate_embedding: true,
            extract_entities: true,
            extract_relations: true,
            search_threshold: 0.7,
            search_limit: 10,
            geocode: true,
            enrich: true,
            deduplicate: true,
        }
    }

    /// Disable all optional processing — useful for tests or minimal deployments.
    pub fn minimal() -> Self {
        Self {
            generate_embedding: false,
            extract_entities: false,
            extract_relations: false,
            search_threshold: 0.0,
            search_limit: 100,
            geocode: false,
            enrich: false,
            deduplicate: false,
        }
    }
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self::default_agent()
    }
}

/// Unified memory tier discriminator.
///
/// Used by callers that need to identify which tier an entry belongs to
/// without depending on the concrete type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryTier {
    ShortTerm,
    LongTerm,
    Reasoning,
}

impl MemoryTier {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ShortTerm => "short_term",
            Self::LongTerm => "long_term",
            Self::Reasoning => "reasoning",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_entry_defaults_are_sane() {
        let entry = MemoryEntry::new();
        assert_ne!(entry.id, Uuid::nil());
        assert!(entry.embedding.is_none());
        assert!(entry.metadata.is_empty());
        assert!(entry.updated_at.is_none());
    }

    #[test]
    fn memory_entry_builder_chain() {
        let entry = MemoryEntry::with_metadata(Metadata::new()).with_embedding(vec![1.0, 2.0]);
        assert_eq!(entry.embedding, Some(vec![1.0, 2.0]));
    }

    #[test]
    fn memory_entry_touch_sets_updated_at() {
        let mut entry = MemoryEntry::new();
        assert!(entry.updated_at.is_none());
        entry.touch();
        assert!(entry.updated_at.is_some());
    }

    #[test]
    fn memory_config_defaults_match_reference() {
        let cfg = MemoryConfig::default_agent();
        assert!(cfg.generate_embedding);
        assert!(cfg.extract_entities);
        assert!(cfg.search_threshold == 0.7);
        assert!(cfg.search_limit == 10);
    }

    #[test]
    fn memory_config_minimal_disables_everything() {
        let cfg = MemoryConfig::minimal();
        assert!(!cfg.generate_embedding);
        assert!(!cfg.extract_entities);
        assert!(!cfg.extract_relations);
        assert!(!cfg.geocode);
        assert!(!cfg.enrich);
        assert!(!cfg.deduplicate);
    }

    #[test]
    fn memory_tier_as_str() {
        assert_eq!(MemoryTier::ShortTerm.as_str(), "short_term");
        assert_eq!(MemoryTier::LongTerm.as_str(), "long_term");
        assert_eq!(MemoryTier::Reasoning.as_str(), "reasoning");
    }

    #[test]
    fn tool_call_status_reexported() {
        use crate::model::ToolCallStatus;
        assert_eq!(ToolCallStatus::Pending.as_str(), "pending");
        assert_eq!(ToolCallStatus::Success.as_str(), "success");
        assert_eq!(ToolCallStatus::Failure.as_str(), "failure");
        assert_eq!(ToolCallStatus::Error.as_str(), "error");
        assert_eq!(ToolCallStatus::Timeout.as_str(), "timeout");
        assert_eq!(ToolCallStatus::Cancelled.as_str(), "cancelled");
    }
}
