//! Schema management (`graph/schema.py` `SchemaManager`).
//!
//! Constraint, index and vector-index names match the pinned upstream so a
//! graph written by either implementation is readable by the other.

use muninn_core::{MemoryError, MemoryResult};
use neo4rs::BoltType;
use serde_json::Value;

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

    /// Serialize schema upgrades and persist a version only after validation.
    pub async fn setup_all(&self) -> MemoryResult<()> {
        self.run("CREATE CONSTRAINT muninn_schema_id IF NOT EXISTS FOR (s:MuninnSchema) REQUIRE s.id IS UNIQUE").await?;
        let definitions = self.client.execute(
            "SHOW CONSTRAINTS YIELD name, type, entityType, labelsOrTypes, properties WHERE name = 'muninn_schema_id' RETURN name, type, entityType, labelsOrTypes, properties",
            vec![],
        ).await?;
        let definitions = schema_rows(&definitions)?;
        validate_schema_definition(
            &definitions,
            "muninn_schema_id",
            "UNIQUENESS",
            "MuninnSchema",
            &["id"],
        )?;
        // Initialize outside the lock transaction so a crash leaves an explicit
        // retryable v0 marker. MERGE is protected by the verified unique key.
        self.run("MERGE (s:MuninnSchema {id: 'product'}) ON CREATE SET s.version = 0, s.state = 'ready', s.revision = 0").await?;
        let coordinator = self.client.schema_coordinator().await?;
        let mut transaction = coordinator
            .start_txn()
            .await
            .map_err(|e| MemoryError::storage(e.to_string()))?;
        let result = async {
            // Direct property dependency acquires the exclusive node write lock
            // BEFORE reading. All cooperating migrators retain it until commit.
            let mut stream = transaction.execute(neo4rs::query(
                "MATCH (s:MuninnSchema {id: 'product'}) SET s.revision = coalesce(s.revision, 0) + 1 RETURN properties(s) AS marker"
            )).await.map_err(|e| MemoryError::storage(e.to_string()))?;
            let row = stream.next(transaction.handle()).await.map_err(|e| MemoryError::storage(e.to_string()))?
                .ok_or_else(|| MemoryError::storage("Muninn schema marker disappeared"))?;
            let marker = to_json(&column(&row, "marker")?);
            crate::migrations::validate_marker(&marker)?;
            drop(stream);
            self.validate_non_vector_definitions(true).await?;
            self.validate_existing_vector_dimensions().await?;
            self.setup_objects().await?;
            transaction.run(neo4rs::query(
                "MATCH (s:MuninnSchema {id: 'product'}) SET s.version = $version, s.state = 'ready'"
            ).param("version", crate::migrations::SCHEMA_VERSION)).await.map_err(|e| MemoryError::storage(e.to_string()))?;
            Ok(())
        }.await;
        match result {
            Ok(()) => transaction
                .commit()
                .await
                .map_err(|e| MemoryError::storage(e.to_string())),
            Err(error) => {
                transaction.rollback().await.map_err(|e| {
                    MemoryError::storage(format!(
                        "schema failure: {error}; lock rollback failed: {e}"
                    ))
                })?;
                Err(error)
            }
        }
    }

    async fn setup_objects(&self) -> MemoryResult<()> {
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
            self.run(&format!(
                "CREATE POINT INDEX {name} IF NOT EXISTS FOR (n:{label}) ON (n.{property})"
            ))
            .await?;
        }
        for (name, _, _) in INDEXES.iter().chain(TEXT_INDEXES).chain(POINT_INDEXES) {
            self.client
                .execute(
                    "CALL db.awaitIndex($name, 60)",
                    vec![("name", BoltType::from(*name))],
                )
                .await?;
        }
        self.validate_non_vector_schema().await?;
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

    async fn validate_existing_vector_dimensions(&self) -> MemoryResult<()> {
        let names = self
            .client
            .execute("SHOW INDEXES YIELD name, type RETURN name, type", vec![])
            .await?;
        for row in names {
            let name: String = row
                .get("name")
                .map_err(|e| MemoryError::storage(e.to_string()))?;
            let kind: String = row
                .get("type")
                .map_err(|e| MemoryError::storage(e.to_string()))?;
            if VECTOR_INDEXES
                .iter()
                .any(|(required, _, _)| *required == name)
                && kind != "VECTOR"
            {
                return Err(MemoryError::storage(format!("required vector index `{name}` is occupied by {kind}; no product DDL was executed")));
            }
        }
        let inventory = self.vector_index_inventory().await?;
        validate_vector_metadata(&inventory, self.vector_dimensions, true)
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

    async fn validate_non_vector_schema(&self) -> MemoryResult<()> {
        self.validate_non_vector_definitions(false).await
    }

    async fn validate_non_vector_definitions(&self, allow_missing: bool) -> MemoryResult<()> {
        let constraints = self.client.execute(
            "SHOW CONSTRAINTS YIELD name, type, entityType, labelsOrTypes, properties RETURN name, type, entityType, labelsOrTypes, properties",
            Vec::<(&'static str, BoltType)>::new(),
        ).await?;
        let indexes = self.client.execute(
            "SHOW INDEXES YIELD name, type, entityType, labelsOrTypes, properties, state, failureMessage RETURN name, type, entityType, labelsOrTypes, properties, state, failureMessage",
            Vec::<(&'static str, BoltType)>::new(),
        ).await?;
        let convert = |rows: &[neo4rs::Row], keys: &[&str]| -> MemoryResult<Vec<Value>> {
            rows.iter()
                .map(|row| {
                    let mut metadata = serde_json::Map::new();
                    for key in keys {
                        metadata.insert((*key).into(), to_json(&column(row, key)?));
                    }
                    Ok(Value::Object(metadata))
                })
                .collect()
        };
        let constraints = convert(
            &constraints,
            &["name", "type", "entityType", "labelsOrTypes", "properties"],
        )?;
        let indexes = convert(
            &indexes,
            &[
                "name",
                "type",
                "entityType",
                "labelsOrTypes",
                "properties",
                "state",
                "failureMessage",
            ],
        )?;
        let validate_constraint = |name: &str, label: &str, properties: &[&str]| {
            if allow_missing && !constraints.iter().any(|row| row["name"] == name) {
                if indexes.iter().any(|row| row["name"] == name) {
                    return Err(MemoryError::storage(format!("required constraint `{name}` is occupied by an index; no product DDL was executed")));
                }
                return Ok(());
            }
            validate_schema_definition(&constraints, name, "UNIQUENESS", label, properties)
        };
        for (name, label, property) in CONSTRAINTS {
            validate_constraint(name, label, &[property])?;
        }
        validate_constraint(
            "conversation_owner_session",
            "Conversation",
            &["owner_scope", "session_id"],
        )?;
        validate_constraint(
            "message_operation",
            "Message",
            &["operation_conversation_id", "operation_key"],
        )?;
        for (definitions, kind) in [
            (INDEXES, "RANGE"),
            (TEXT_INDEXES, "TEXT"),
            (POINT_INDEXES, "POINT"),
        ] {
            for (name, label, property) in definitions {
                if allow_missing && !indexes.iter().any(|row| row["name"] == *name) {
                    continue;
                }
                validate_schema_definition(&indexes, name, kind, label, &[property])?;
                let row = indexes
                    .iter()
                    .find(|row| row["name"].as_str() == Some(*name));
                if !row.is_some_and(|row| {
                    (row["state"] == "ONLINE" || (allow_missing && row["state"] == "POPULATING"))
                        && row["failureMessage"] == ""
                }) {
                    return Err(MemoryError::storage(format!("required index `{name}` is failed or unavailable; inspect SHOW INDEXES; no objects were removed or rebuilt")));
                }
            }
        }
        Ok(())
    }

    async fn run(&self, cypher: &str) -> MemoryResult<()> {
        self.client
            .execute(cypher, Vec::<(&'static str, BoltType)>::new())
            .await
            .map(|_| ())
    }
}

fn schema_rows(rows: &[neo4rs::Row]) -> MemoryResult<Vec<Value>> {
    rows.iter()
        .map(|row| {
            let mut value = serde_json::Map::new();
            for key in ["name", "type", "entityType", "labelsOrTypes", "properties"] {
                value.insert(key.to_owned(), to_json(&column(row, key)?));
            }
            Ok(Value::Object(value))
        })
        .collect()
}

fn validate_schema_definition(
    rows: &[Value],
    name: &str,
    kind: &str,
    label: &str,
    properties: &[&str],
) -> MemoryResult<()> {
    let row = rows.iter().find(|row| row["name"].as_str() == Some(name));
    if !row.is_some_and(|row| {
        row["type"] == kind
            && row["entityType"] == "NODE"
            && row["labelsOrTypes"] == serde_json::json!([label])
            && row["properties"] == serde_json::json!(properties)
    }) {
        return Err(MemoryError::storage(format!(
            "required schema object `{name}` is missing or incompatible; expected {kind} on {label} {properties:?}; inspect SHOW CONSTRAINTS and SHOW INDEXES; no objects were removed or rebuilt"
        )));
    }
    Ok(())
}

/// Validate configuration separately so freshly created POPULATING indexes can
/// be awaited, but existing incompatible indexes are never accepted or rebuilt.
fn validate_vector_index_configuration(rows: &[Value], expected: usize) -> MemoryResult<()> {
    validate_vector_metadata(rows, expected, false)
}

fn validate_vector_metadata(
    rows: &[Value],
    expected: usize,
    allow_missing: bool,
) -> MemoryResult<()> {
    for (name, label, property) in VECTOR_INDEXES {
        let row = rows.iter().find(|row| row["name"].as_str() == Some(*name));
        if allow_missing && row.is_none() {
            continue;
        }
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

    #[test]
    fn named_constraint_requires_exact_definition() {
        let mut row = json!({"name":"conversation_id", "type":"UNIQUENESS",
            "entityType":"NODE", "labelsOrTypes":["Conversation"], "properties":["id"]});
        assert!(validate_schema_definition(
            &[row.clone()],
            "conversation_id",
            "UNIQUENESS",
            "Conversation",
            &["id"]
        )
        .is_ok());
        row["properties"] = json!(["wrong"]);
        assert!(validate_schema_definition(
            &[row],
            "conversation_id",
            "UNIQUENESS",
            "Conversation",
            &["id"]
        )
        .is_err());
        assert!(validate_schema_definition(
            &[],
            "conversation_id",
            "UNIQUENESS",
            "Conversation",
            &["id"]
        )
        .is_err());
    }

    #[test]
    fn composite_constraint_and_index_definitions_are_exact() {
        for (name, kind, label, properties) in [
            (
                "conversation_owner_session",
                "UNIQUENESS",
                "Conversation",
                vec!["owner_scope", "session_id"],
            ),
            (
                "message_operation",
                "UNIQUENESS",
                "Message",
                vec!["operation_conversation_id", "operation_key"],
            ),
            (
                "conversation_session_idx",
                "RANGE",
                "Conversation",
                vec!["session_id"],
            ),
            (
                "message_content_text_idx",
                "TEXT",
                "Message",
                vec!["content"],
            ),
        ] {
            let row = json!({"name":name, "type":kind, "entityType":"NODE",
                "labelsOrTypes":[label], "properties":properties});
            assert!(validate_schema_definition(
                std::slice::from_ref(&row),
                name,
                kind,
                label,
                &properties
            )
            .is_ok());
            for (key, invalid) in [
                ("type", json!("WRONG")),
                ("entityType", json!("RELATIONSHIP")),
                ("labelsOrTypes", json!(["Wrong"])),
                ("properties", json!(["wrong"])),
            ] {
                let mut incompatible = row.clone();
                incompatible[key] = invalid;
                assert!(
                    validate_schema_definition(&[incompatible], name, kind, label, &properties)
                        .is_err(),
                    "{name}: {key}"
                );
            }
            if properties.len() > 1 {
                let mut reversed = properties.clone();
                reversed.reverse();
                let mut incompatible = row.clone();
                incompatible["properties"] = json!(reversed);
                assert!(validate_schema_definition(
                    &[incompatible],
                    name,
                    kind,
                    label,
                    &properties
                )
                .is_err());
            }
        }
    }

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
