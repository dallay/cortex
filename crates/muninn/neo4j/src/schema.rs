//! Schema management (`graph/schema.py` `SchemaManager`).
//!
//! Constraint, index and vector-index names match the pinned upstream so a
//! graph written by either implementation is readable by the other.

use muninn_core::{MemoryError, MemoryResult};
use neo4rs::BoltType;
use serde_json::Value;
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
        for (indexes, kind, provider) in [
            (INDEXES, "RANGE", "range-1.0"),
            (TEXT_INDEXES, "TEXT", "text-2.0"),
        ] {
            for (name, label, property) in indexes {
                let ddl = format!(
                    "CREATE {kind} INDEX {name} IF NOT EXISTS FOR (n:{label}) ON (n.{property})"
                );
                if let Err(error) = self.run(&ddl).await {
                    // IF NOT EXISTS can race in Neo4j 5.26. The error alone is
                    // insufficient: reread schema before accepting equivalence.
                    if !error
                        .to_string()
                        .contains("Neo.ClientError.Schema.EquivalentSchemaRuleAlreadyExists")
                        || !self
                            .equivalent_index(name, label, property, kind, provider)
                            .await?
                    {
                        return Err(MemoryError::storage(format!("index {name}: {error}")));
                    }
                }
            }
        }
        for (name, label, property) in VECTOR_INDEXES {
            let ddl = format!(
                "CREATE VECTOR INDEX {name} IF NOT EXISTS FOR (n:{label}) ON (n.{property}) \
                 OPTIONS {{ indexConfig: {{ `vector.dimensions`: {}, \
                 `vector.similarity_function`: 'cosine' }} }}",
                self.vector_dimensions
            );
            if let Err(error) = self.run(&ddl).await {
                // Neo4j 5.26 can report this race even with IF NOT EXISTS.
                // Defer acceptance until the complete metadata inventory is
                // checked below; an equivalent name alone is not sufficient.
                if !error
                    .to_string()
                    .contains("Neo.ClientError.Schema.EquivalentSchemaRuleAlreadyExists")
                {
                    return Err(MemoryError::storage(format!(
                        "required vector index `{name}` creation failed (expected {} dimensions, cosine): {error}; verify Neo4j vector support, permissions and existing schema; no automatic rebuild was attempted",
                        self.vector_dimensions
                    )));
                }
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
        let inventory = self.vector_index_inventory().await?;
        validate_vector_index_configuration(&inventory, self.vector_dimensions)?;
        for (name, _, _) in VECTOR_INDEXES {
            self.client.execute(
                "CALL db.awaitIndex($name, 60)",
                vec![("name", BoltType::from(*name))],
            ).await.map_err(|err| MemoryError::storage(format!(
                "required vector index `{name}` did not become ONLINE within 60 seconds: {err}; inspect SHOW VECTOR INDEXES YIELD *; no automatic rebuild was attempted"
            )))?;
        }
        validate_vector_indexes(
            &self.vector_index_inventory().await?,
            self.vector_dimensions,
        )
    }

    /// Legacy best-effort wait for all indexes. `setup_all` instead requires
    /// each managed vector index to become ONLINE and propagates failures.
    pub async fn await_indexes(&self) {
        let _ = self
            .client
            .execute("CALL db.awaitIndexes(30)", Vec::new())
            .await;
    }

    /// Full Neo4j 5.26 VECTOR inventory, including configuration and failure state.
    pub async fn vector_index_inventory(&self) -> MemoryResult<Vec<Value>> {
        let rows = self.client.execute(
            "SHOW VECTOR INDEXES YIELD name, state, type, entityType, labelsOrTypes, properties, indexProvider, options, failureMessage RETURN name, state, type, entityType, labelsOrTypes, properties, indexProvider, options, failureMessage ORDER BY name",
            Vec::new(),
        ).await.map_err(|err| MemoryError::storage(format!(
            "cannot inspect required vector index metadata: {err}; verify Neo4j 5.26 support and SHOW INDEX privileges"
        )))?;
        rows.iter()
            .map(|row| {
                let mut metadata = serde_json::Map::new();
                for key in [
                    "name",
                    "state",
                    "type",
                    "entityType",
                    "labelsOrTypes",
                    "properties",
                    "indexProvider",
                    "options",
                    "failureMessage",
                ] {
                    metadata.insert(key.into(), to_json(&column(row, key)?));
                }
                Ok(Value::Object(metadata))
            })
            .collect()
    }

    /// Require every managed vector index to be compatible and ONLINE.
    pub async fn validate_vector_index_dimensions(&self, expected: usize) -> MemoryResult<()> {
        validate_vector_indexes(&self.vector_index_inventory().await?, expected)
    }

    async fn equivalent_index(
        &self,
        name: &str,
        label: &str,
        property: &str,
        kind: &str,
        provider: &str,
    ) -> MemoryResult<bool> {
        let rows = self.client.execute(
            "SHOW INDEXES YIELD name, type, entityType, labelsOrTypes, properties, indexProvider, options, state WHERE name = $name RETURN type, entityType, labelsOrTypes, properties, indexProvider, options, state",
            vec![("name", BoltType::from(name))],
        ).await?;
        if rows.len() != 1 {
            return Ok(false);
        }
        let value = |key| column(&rows[0], key).map(|value| to_json(&value));
        Ok(value("type")? == kind
            && value("entityType")? == "NODE"
            && value("labelsOrTypes")? == serde_json::json!([label])
            && value("properties")? == serde_json::json!([property])
            && value("indexProvider")? == provider
            && value("options")?
                == serde_json::json!({
                    "indexProvider": provider, "indexConfig": {}
                })
            && matches!(value("state")?.as_str(), Some("ONLINE" | "POPULATING")))
    }

    async fn run(&self, cypher: &str) -> MemoryResult<()> {
        self.client
            .execute(cypher, Vec::<(&'static str, BoltType)>::new())
            .await
            .map(|_| ())
    }
}

/// Validate configuration separately so freshly created POPULATING indexes can
/// be awaited, but existing incompatible indexes are never accepted or rebuilt.
fn validate_vector_index_configuration(rows: &[Value], expected: usize) -> MemoryResult<()> {
    for (name, label, property) in VECTOR_INDEXES {
        let row = rows.iter().find(|row| row["name"].as_str() == Some(*name));
        let valid = row.is_some_and(|row| {
            row["type"] == "VECTOR"
                && row["entityType"] == "NODE"
                && row["labelsOrTypes"] == serde_json::json!([label])
                && row["properties"] == serde_json::json!([property])
                && matches!(
                    row["indexProvider"].as_str(),
                    Some("vector-1.0" | "vector-2.0")
                )
                && row
                    .pointer("/options/indexConfig/vector.dimensions")
                    .and_then(Value::as_u64)
                    .and_then(|n| usize::try_from(n).ok())
                    == Some(expected)
                && row
                    .pointer("/options/indexConfig/vector.similarity_function")
                    .and_then(Value::as_str)
                    .is_some_and(|similarity| similarity.eq_ignore_ascii_case("cosine"))
                && matches!(row["state"].as_str(), Some("ONLINE" | "POPULATING"))
                && row["failureMessage"].as_str() == Some("")
        });
        if !valid {
            return Err(MemoryError::storage(format!(
                "required vector index `{name}` is missing or incompatible: expected NODE :{label}({property}), vector.dimensions={expected}, cosine, supported vector provider and usable state; observed {}; inspect SHOW VECTOR INDEXES YIELD * and explicitly repair the schema or select the matching embedding model; no automatic drop/rebuild",
                row.map_or_else(|| "missing".into(), Value::to_string)
            )));
        }
    }
    Ok(())
}

fn validate_vector_indexes(rows: &[Value], expected: usize) -> MemoryResult<()> {
    validate_vector_index_configuration(rows, expected)?;
    for (name, _, _) in VECTOR_INDEXES {
        if let Some(row) = rows.iter().find(|row| row["name"].as_str() == Some(*name)) {
            if row["state"] != "ONLINE" {
                return Err(MemoryError::storage(format!(
                    "required vector index `{name}` is not ONLINE (expected vector.dimensions={expected}); observed state {}; inspect SHOW VECTOR INDEXES YIELD * and retry after population completes",
                    row["state"]
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod vector_metadata_tests {
    use super::*;
    use serde_json::{json, Value};

    fn inventory() -> Vec<Value> {
        VECTOR_INDEXES.iter().map(|(name, label, property)| json!({
            "name": name, "state": "ONLINE", "type": "VECTOR", "entityType": "NODE",
            "labelsOrTypes": [label], "properties": [property], "indexProvider": "vector-2.0",
            "options": {"indexConfig": {"vector.dimensions": 384, "vector.similarity_function": "cosine"}},
            "failureMessage": ""
        })).collect()
    }

    #[test]
    fn vector_metadata_accepts_only_compatible_online_inventory() {
        assert!(validate_vector_indexes(&inventory(), 384).is_ok());
    }

    #[test]
    fn vector_metadata_rejects_missing_required_index() {
        let mut rows = inventory();
        rows.remove(0);
        let error = validate_vector_indexes(&rows, 384).unwrap_err().to_string();
        assert!(error.contains("message_embedding"));
        assert!(error.contains("missing"));
    }

    #[test]
    fn vector_metadata_rejects_incompatible_or_unusable_indexes() {
        for (field, value) in [
            ("state", json!("POPULATING")),
            ("state", json!("FAILED")),
            ("type", json!("RANGE")),
            ("entityType", json!("RELATIONSHIP")),
            ("labelsOrTypes", json!(["Wrong"])),
            ("properties", json!(["wrong"])),
            ("indexProvider", json!("unknown")),
            (
                "options",
                json!({"indexConfig": {"vector.dimensions": 1536, "vector.similarity_function": "cosine"}}),
            ),
            (
                "options",
                json!({"indexConfig": {"vector.dimensions": 384, "vector.similarity_function": "euclidean"}}),
            ),
            ("options", json!({})),
        ] {
            let mut rows = inventory();
            rows[0][field] = value;
            let error = validate_vector_indexes(&rows, 384).unwrap_err().to_string();
            assert!(error.contains("message_embedding"), "{error}");
            assert!(error.contains("384"), "{error}");
        }
    }
}
