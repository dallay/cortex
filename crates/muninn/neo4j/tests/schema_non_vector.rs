//! Run each scenario against its own NEW disposable Neo4j fixture.
//! Never use an application database. The test does not remove schema or data.
use muninn_neo4j::{Neo4jClient, Neo4jConfig, SchemaManager};
use neo4rs::BoltType;

async fn execute(client: &Neo4jClient, query: &str) -> Vec<neo4rs::Row> {
    client
        .execute(query, Vec::<(&'static str, BoltType)>::new())
        .await
        .expect("fixture query")
}

fn definition(row: neo4rs::Row) -> String {
    serde_json::json!({
        "name": row.get::<String>("name").unwrap(),
        "type": row.get::<String>("type").unwrap(),
        "entityType": row.get::<String>("entityType").unwrap(),
        "labelsOrTypes": row.get::<Vec<String>>("labelsOrTypes").unwrap(),
        "properties": row.get::<Vec<String>>("properties").unwrap(),
    })
    .to_string()
}

async fn inventory(client: &Neo4jClient) -> Vec<String> {
    execute(client, "SHOW INDEXES YIELD name, type, entityType, labelsOrTypes, properties WHERE type <> 'LOOKUP' RETURN name, type, entityType, labelsOrTypes, properties ORDER BY name")
        .await.into_iter().map(definition).collect()
}

/// Expected managed schema of a clean fixture, pinned to the upstream contract.
///
/// Neo4j exposes the backing index of every uniqueness constraint as a RANGE
/// index with the same name, so the constraint list and the index list overlap
/// by design.
const EXPECTED_CONSTRAINTS: &[(&str, &str, &[&str])] = &[
    ("muninn_schema_id", "MuninnSchema", &["id"]),
    ("conversation_id", "Conversation", &["id"]),
    ("message_id", "Message", &["id"]),
    ("entity_id", "Entity", &["id"]),
    ("preference_id", "Preference", &["id"]),
    ("fact_id", "Fact", &["id"]),
    ("reasoning_trace_id", "ReasoningTrace", &["id"]),
    ("reasoning_step_id", "ReasoningStep", &["id"]),
    ("tool_name", "Tool", &["name"]),
    ("tool_call_id", "ToolCall", &["id"]),
    ("user_identifier", "User", &["identifier"]),
    ("consolidation_run_id", "ConsolidationRun", &["id"]),
    ("memory_read_audit_id", "MemoryReadAudit", &["id"]),
    ("ontology_id", "Ontology", &["id"]),
    ("ontology_version_id", "OntologyVersion", &["id"]),
    ("ontology_lock_id", "OntologyLock", &["id"]),
    (
        "conversation_owner_session",
        "Conversation",
        &["owner_scope", "session_id"],
    ),
    (
        "message_operation",
        "Message",
        &["operation_conversation_id", "operation_key"],
    ),
];

const EXPECTED_INDEXES: &[(&str, &str, &str, &[&str])] = &[
    (
        "conversation_session_idx",
        "RANGE",
        "Conversation",
        &["session_id"],
    ),
    ("message_timestamp_idx", "RANGE", "Message", &["timestamp"]),
    ("message_role_idx", "RANGE", "Message", &["role"]),
    ("entity_type_idx", "RANGE", "Entity", &["type"]),
    ("entity_name_idx", "RANGE", "Entity", &["name"]),
    (
        "entity_canonical_idx",
        "RANGE",
        "Entity",
        &["canonical_name"],
    ),
    ("entity_name_key_idx", "RANGE", "Entity", &["name_key"]),
    (
        "preference_category_idx",
        "RANGE",
        "Preference",
        &["category"],
    ),
    (
        "trace_session_idx",
        "RANGE",
        "ReasoningTrace",
        &["session_id"],
    ),
    ("trace_success_idx", "RANGE", "ReasoningTrace", &["success"]),
    (
        "trace_error_kind_idx",
        "RANGE",
        "ReasoningTrace",
        &["error_kind"],
    ),
    ("tool_call_status_idx", "RANGE", "ToolCall", &["status"]),
    (
        "conversation_archived_idx",
        "RANGE",
        "Conversation",
        &["archived"],
    ),
    (
        "consolidation_run_kind_idx",
        "RANGE",
        "ConsolidationRun",
        &["kind"],
    ),
    (
        "memory_read_audit_kind_idx",
        "RANGE",
        "MemoryReadAudit",
        &["kind"],
    ),
    ("ontology_name_idx", "RANGE", "Ontology", &["name"]),
    (
        "entity_surface_keys_idx",
        "TEXT",
        "Entity",
        &["surface_keys"],
    ),
    ("entity_name_key_text_idx", "TEXT", "Entity", &["name_key"]),
    ("message_embedding_idx", "VECTOR", "Message", &["embedding"]),
    ("entity_embedding_idx", "VECTOR", "Entity", &["embedding"]),
    (
        "preference_embedding_idx",
        "VECTOR",
        "Preference",
        &["embedding"],
    ),
    ("fact_embedding_idx", "VECTOR", "Fact", &["embedding"]),
    (
        "task_embedding_idx",
        "VECTOR",
        "ReasoningTrace",
        &["task_embedding"],
    ),
    (
        "step_embedding_idx",
        "VECTOR",
        "ReasoningStep",
        &["embedding"],
    ),
    ("entity_location_idx", "POINT", "Entity", &["location"]),
];

/// Assert the exact managed schema of a clean fixture.
///
/// The inventory must contain the pinned managed objects and nothing else,
/// except the backing indexes Neo4j creates for uniqueness constraints.
async fn assert_clean_inventory(client: &Neo4jClient) {
    let constraints = execute(
        client,
        "SHOW CONSTRAINTS YIELD name, type, entityType, labelsOrTypes, properties WHERE type = 'UNIQUENESS' RETURN name, labelsOrTypes, properties ORDER BY name",
    )
    .await;
    let mut actual: Vec<(String, String, Vec<String>)> = constraints
        .into_iter()
        .map(|row| {
            let name = row.get::<String>("name").unwrap();
            let label = row.get::<Vec<String>>("labelsOrTypes").unwrap()[0].clone();
            let properties = row.get::<Vec<String>>("properties").unwrap();
            (name, label, properties)
        })
        .collect();
    let mut expected: Vec<(String, String, Vec<String>)> = EXPECTED_CONSTRAINTS
        .iter()
        .map(|(name, label, properties)| {
            (
                (*name).to_string(),
                (*label).to_string(),
                properties
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
            )
        })
        .collect();
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected, "managed constraint inventory differs");

    let indexes = execute(
        client,
        "SHOW INDEXES YIELD name, type, entityType, labelsOrTypes, properties WHERE type <> 'LOOKUP' RETURN name, type, labelsOrTypes, properties ORDER BY name",
    )
    .await;
    let constraint_names: Vec<&str> = EXPECTED_CONSTRAINTS
        .iter()
        .map(|(name, _, _)| *name)
        .collect();
    let mut actual: Vec<(String, String, String, Vec<String>)> = Vec::new();
    for row in indexes {
        let name = row.get::<String>("name").unwrap();
        if name == "foreign_schema_sentinel" {
            continue;
        }
        // Backing indexes of uniqueness constraints share the constraint name.
        if constraint_names.contains(&name.as_str())
            && row.get::<String>("type").unwrap() == "RANGE"
        {
            continue;
        }
        let label = row.get::<Vec<String>>("labelsOrTypes").unwrap()[0].clone();
        let properties = row.get::<Vec<String>>("properties").unwrap();
        actual.push((name, row.get::<String>("type").unwrap(), label, properties));
    }
    let mut expected: Vec<(String, String, String, Vec<String>)> = EXPECTED_INDEXES
        .iter()
        .map(|(name, kind, label, properties)| {
            (
                (*name).to_string(),
                (*kind).to_string(),
                (*label).to_string(),
                properties
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
            )
        })
        .collect();
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected, "managed index inventory differs");
}

#[tokio::test]
#[ignore = "requires explicit opt-in and a separate NEW disposable fixture per scenario"]
async fn non_vector_schema_preserves_data_and_rejects_incompatible_objects() {
    assert_eq!(
        std::env::var("MUNINN_SCHEMA_ISOLATED_TEST").as_deref(),
        Ok("1")
    );
    let scenario = std::env::var("MUNINN_SCHEMA_SCENARIO").expect("explicit scenario required");
    let (name, ddl) = match scenario.as_str() {
        "clean" => (None, None),
        "constraint" => (Some("conversation_id"), Some("CREATE CONSTRAINT conversation_id FOR (n:Wrong) REQUIRE n.wrong IS UNIQUE")),
        "composite" => (Some("message_operation"), Some("CREATE CONSTRAINT message_operation FOR (n:Message) REQUIRE (n.operation_key, n.operation_conversation_id) IS UNIQUE")),
        "range" => (Some("conversation_session_idx"), Some("CREATE RANGE INDEX conversation_session_idx FOR (n:Conversation) ON (n.wrong)")),
        "text" => (Some("entity_name_key_text_idx"), Some("CREATE TEXT INDEX entity_name_key_text_idx FOR (n:Wrong) ON (n.content)")),
        _ => panic!("unknown fixture scenario"),
    };
    let client = Neo4jClient::connect(&Neo4jConfig {
        uri: std::env::var("MUNINN_SCHEMA_TEST_URI").expect("fixture URI required"),
        username: "neo4j".into(),
        password: std::env::var("MUNINN_SCHEMA_TEST_PASSWORD").expect("fixture password required"),
        ..Default::default()
    })
    .await
    .expect("connect to disposable fixture");
    assert!(
        execute(&client, "MATCH (n) RETURN n LIMIT 1")
            .await
            .is_empty(),
        "fixture must contain no nodes"
    );
    assert!(
        execute(&client, "SHOW CONSTRAINTS").await.is_empty(),
        "fixture must contain no constraints"
    );
    let indexes = execute(
        &client,
        "SHOW INDEXES YIELD type WHERE type <> 'LOOKUP' RETURN type",
    )
    .await;
    assert!(
        indexes.is_empty(),
        "fixture must contain no application indexes"
    );
    execute(&client, "CREATE (n:SchemaSentinel {value: 'preserve'})").await;
    execute(
        &client,
        "CREATE RANGE INDEX foreign_schema_sentinel FOR (n:SchemaSentinel) ON (n.value)",
    )
    .await;
    if let Some(ddl) = ddl {
        execute(&client, ddl).await;
    }
    let before = inventory(&client).await;
    let constraints_before: Vec<String> = execute(&client, "SHOW CONSTRAINTS YIELD name, type, entityType, labelsOrTypes, properties RETURN name, type, entityType, labelsOrTypes, properties ORDER BY name")
        .await.into_iter().map(definition).collect();
    println!("scenario={scenario}; indexes_before={before:#?}; constraints_before={constraints_before:#?}");
    let schema = SchemaManager::new(&client, 3);
    if let Some(name) = name {
        let error = schema
            .setup_all()
            .await
            .expect_err("incompatible object must be rejected")
            .to_string();
        assert!(error.contains(name), "error must identify {name}: {error}");
        println!("rejected={error}");
        // Each original index must still exist with its original definition.
        let after = inventory(&client).await;
        for object in &before {
            assert!(after.contains(object), "original index changed: {object}");
        }
        let constraints_after: Vec<String> = execute(&client, "SHOW CONSTRAINTS YIELD name, type, entityType, labelsOrTypes, properties RETURN name, type, entityType, labelsOrTypes, properties ORDER BY name")
            .await.into_iter().map(definition).collect();
        for object in &constraints_before {
            assert!(
                constraints_after.contains(object),
                "original constraint changed: {object}"
            );
        }
    } else {
        schema.setup_all().await.expect("clean setup");
        let first = inventory(&client).await;
        schema.setup_all().await.expect("idempotent setup");
        assert_eq!(first, inventory(&client).await);
        assert_clean_inventory(&client).await;
    }
    let sentinel = execute(&client, "MATCH (n:SchemaSentinel) RETURN n.value AS value").await;
    assert_eq!(sentinel.len(), 1);
    assert_eq!(sentinel[0].get::<String>("value").unwrap(), "preserve");
    println!("indexes_after={:#?}", inventory(&client).await);
}
