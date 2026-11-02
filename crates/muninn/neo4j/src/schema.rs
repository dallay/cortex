//! Schema management (`graph/schema.py` `SchemaManager`).
//!
//! Constraint, index and vector-index names match the pinned upstream so a
//! graph written by either implementation is readable by the other.

use muninn_core::{MemoryError, MemoryResult};
use neo4rs::BoltType;
use tracing::warn;

use crate::bolt::{column, to_json};
use crate::client::Neo4jClient;

/// Upstream `DEFAULT_VECTOR_DIMENSIONS`.
pub const DEFAULT_VECTOR_DIMENSIONS: usize = 1536;

const CONSTRAINTS: &[(&str, &str, &str)] = &[
    ("conversation_id", "Conversation", "id"),
    ("message_id", "Message", "id"),
    ("entity_id", "Entity", "id"),
    ("preference_id", "Preference", "id"),
    ("fact_id", "Fact", "id"),
    ("reasoning_trace_id", "ReasoningTrace", "id"),
    ("reasoning_step_id", "ReasoningStep", "id"),
    ("tool_name", "Tool", "name"),
    ("tool_call_id", "ToolCall", "id"),
    ("user_identifier", "User", "identifier"),
    ("consolidation_run_id", "ConsolidationRun", "id"),
    ("memory_read_audit_id", "MemoryReadAudit", "id"),
    ("ontology_id", "Ontology", "id"),
    ("ontology_version_id", "OntologyVersion", "id"),
    ("ontology_lock_id", "OntologyLock", "id"),
];

const INDEXES: &[(&str, &str, &str)] = &[
    ("conversation_session_idx", "Conversation", "session_id"),
    ("message_timestamp_idx", "Message", "timestamp"),
    ("message_role_idx", "Message", "role"),
    ("entity_type_idx", "Entity", "type"),
    ("entity_name_idx", "Entity", "name"),
    ("entity_canonical_idx", "Entity", "canonical_name"),
    ("entity_name_key_idx", "Entity", "name_key"),
    ("preference_category_idx", "Preference", "category"),
    ("trace_session_idx", "ReasoningTrace", "session_id"),
    ("trace_success_idx", "ReasoningTrace", "success"),
    ("trace_error_kind_idx", "ReasoningTrace", "error_kind"),
    ("tool_call_status_idx", "ToolCall", "status"),
    ("conversation_archived_idx", "Conversation", "archived"),
    ("consolidation_run_kind_idx", "ConsolidationRun", "kind"),
    ("memory_read_audit_kind_idx", "MemoryReadAudit", "kind"),
    ("ontology_name_idx", "Ontology", "name"),
];

const TEXT_INDEXES: &[(&str, &str, &str)] = &[
    ("entity_surface_keys_idx", "Entity", "surface_keys"),
    ("entity_name_key_text_idx", "Entity", "name_key"),
];

/// Vector indexes managed by the library (`_MANAGED_VECTOR_INDEXES`).
pub const VECTOR_INDEXES: &[(&str, &str, &str)] = &[
    ("message_embedding_idx", "Message", "embedding"),
    ("entity_embedding_idx", "Entity", "embedding"),
    ("preference_embedding_idx", "Preference", "embedding"),
    ("fact_embedding_idx", "Fact", "embedding"),
    ("task_embedding_idx", "ReasoningTrace", "task_embedding"),
    ("step_embedding_idx", "ReasoningStep", "embedding"),
];

const POINT_INDEXES: &[(&str, &str, &str)] = &[("entity_location_idx", "Entity", "location")];

pub struct SchemaManager<'a> {
    client: &'a Neo4jClient,
    vector_dimensions: usize,
}

impl<'a> SchemaManager<'a> {
    pub const fn new(client: &'a Neo4jClient, vector_dimensions: usize) -> Self {
        Self {
            client,
            vector_dimensions,
        }
    }

    /// Create every constraint and index (`setup_all`). Idempotent.
    pub async fn setup_all(&self) -> MemoryResult<()> {
        for (name, label, property) in CONSTRAINTS {
            self.run(&format!(
                "CREATE CONSTRAINT {name} IF NOT EXISTS FOR (n:{label}) REQUIRE n.{property} IS UNIQUE"
            ))
            .await
            .map_err(|err| MemoryError::storage(format!("constraint {name}: {err}")))?;
        }
        self.run("CREATE CONSTRAINT conversation_owner_session IF NOT EXISTS FOR (c:Conversation) REQUIRE (c.owner_scope, c.session_id) IS UNIQUE").await?;
        self.run("CREATE CONSTRAINT message_operation IF NOT EXISTS FOR (m:Message) REQUIRE (m.operation_conversation_id, m.operation_key) IS UNIQUE").await?;
        for (name, label, property) in INDEXES {
            self.run(&format!(
                "CREATE INDEX {name} IF NOT EXISTS FOR (n:{label}) ON (n.{property})"
            ))
            .await
            .map_err(|err| MemoryError::storage(format!("index {name}: {err}")))?;
        }
        for (name, label, property) in TEXT_INDEXES {
            self.run(&format!(
                "CREATE TEXT INDEX {name} IF NOT EXISTS FOR (n:{label}) ON (n.{property})"
            ))
            .await
            .map_err(|err| MemoryError::storage(format!("text index {name}: {err}")))?;
        }
        for (name, label, property) in VECTOR_INDEXES {
            let ddl = format!(
                "CREATE VECTOR INDEX {name} IF NOT EXISTS FOR (n:{label}) ON (n.{property}) \
                 OPTIONS {{ indexConfig: {{ `vector.dimensions`: {}, \
                 `vector.similarity_function`: 'cosine' }} }}",
                self.vector_dimensions
            );
            // Upstream tolerates servers without vector support.
            if let Err(err) = self.run(&ddl).await {
                warn!(index = name, error = %err, "vector index creation failed");
            }
        }
        for (name, label, property) in POINT_INDEXES {
            if let Err(err) = self
                .run(&format!(
                    "CREATE POINT INDEX {name} IF NOT EXISTS FOR (n:{label}) ON (n.{property})"
                ))
                .await
            {
                warn!(index = name, error = %err, "point index creation failed");
            }
        }
        self.await_indexes().await;
        Ok(())
    }

    /// Fail when a managed vector index exists with other dimensions
    /// (`validate_vector_index_dimensions`), instead of silently mixing
    /// incompatible vectors.
    pub async fn validate_vector_index_dimensions(&self, expected: usize) -> MemoryResult<()> {
        let rows = match self
            .client
            .execute(
                "SHOW VECTOR INDEXES YIELD name, options RETURN name AS name, options AS options",
                Vec::new(),
            )
            .await
        {
            Ok(rows) => rows,
            Err(err) => {
                warn!(error = %err, "cannot validate vector index dimensions");
                return Ok(());
            }
        };
        let mut mismatches = Vec::new();
        for row in rows {
            let name = column(&row, "name").map(|v| to_json(&v))?;
            let Some(name) = name.as_str() else { continue };
            if !VECTOR_INDEXES
                .iter()
                .any(|(managed, _, _)| *managed == name)
            {
                continue;
            }
            let options = to_json(&column(&row, "options")?);
            let actual = options
                .pointer("/indexConfig/vector.dimensions")
                .and_then(serde_json::Value::as_u64);
            if let Some(actual) = actual {
                if usize::try_from(actual).ok() != Some(expected) {
                    mismatches.push(format!(
                        "index '{name}': expected {expected}, found {actual}"
                    ));
                }
            }
        }
        if mismatches.is_empty() {
            Ok(())
        } else {
            Err(MemoryError::DimensionMismatch(format!(
                "{}. The embedding model changed since the indexes were created; drop and \
                 recreate the indexes or revert to the previous model.",
                mismatches.join("; ")
            )))
        }
    }

    async fn await_indexes(&self) {
        if let Err(err) = self.run("CALL db.awaitIndexes(60)").await {
            warn!(error = %err, "waiting for indexes failed");
        }
    }

    async fn run(&self, cypher: &str) -> MemoryResult<()> {
        self.client
            .execute(cypher, Vec::<(&'static str, BoltType)>::new())
            .await
            .map(|_| ())
    }
}
