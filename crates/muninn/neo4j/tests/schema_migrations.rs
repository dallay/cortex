//! Requires a NEW, empty, disposable fixture. Never target application data.
use muninn_neo4j::{Neo4jClient, Neo4jConfig, SchemaManager};

#[tokio::test]
#[ignore = "requires an explicitly authorized empty disposable Neo4j fixture"]
async fn schema_migration_contract() {
    assert_eq!(
        std::env::var("MUNINN_SCHEMA_ISOLATED_TEST").as_deref(),
        Ok("1")
    );
    let config = Neo4jConfig {
        uri: std::env::var("MUNINN_SCHEMA_TEST_URI").expect("fixture URI"),
        password: std::env::var("MUNINN_SCHEMA_TEST_PASSWORD").expect("fixture password"),
        max_connections: 1,
        ..Neo4jConfig::default()
    };
    let client = Neo4jClient::connect(&config).await.unwrap();
    let rows = client
        .execute("MATCH (n) RETURN count(n) AS count", vec![])
        .await
        .unwrap();
    assert_eq!(
        rows[0].get::<i64>("count").unwrap(),
        0,
        "fixture must be empty"
    );
    let bootstrap_other = Neo4jClient::connect(&config).await.unwrap();
    let initial_a = SchemaManager::new(&client, 384);
    let initial_b = SchemaManager::new(&bootstrap_other, 384);
    let (a, b) = tokio::join!(initial_a.setup_all(), initial_b.setup_all());
    a.unwrap();
    b.unwrap();
    let rows = client
        .execute(
            "MATCH (s:MuninnSchema {id: 'product'}) RETURN s.version AS version, s.state AS state",
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(
        rows.len(),
        1,
        "setup must persist an independent schema version"
    );
    assert_eq!(rows[0].get::<i64>("version").unwrap(), 1);
    assert_eq!(rows[0].get::<String>("state").unwrap(), "ready");

    client
        .execute("CREATE (:MigrationSentinel {value: 'preserved'})", vec![])
        .await
        .unwrap();
    let other = Neo4jClient::connect(&config).await.unwrap();
    let first = SchemaManager::new(&client, 384);
    let second = SchemaManager::new(&other, 384);
    let (a, b) = tokio::join!(first.setup_all(), second.setup_all());
    a.unwrap();
    b.unwrap();
    let rows = client
        .execute(
            "MATCH (s:MuninnSchema {id: 'product'}) RETURN s.revision AS revision",
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(rows[0].get::<i64>("revision").unwrap(), 4);

    // A future or incomplete marker must reject before filling missing DDL.
    client
        .execute("DROP INDEX conversation_session_idx", vec![])
        .await
        .unwrap();
    for mutation in [
        "MATCH (s:MuninnSchema) SET s.version = 2",
        "MATCH (s:MuninnSchema) SET s.version = 1, s.state = 'migrating'",
    ] {
        client.execute(mutation, vec![]).await.unwrap();
        assert!(SchemaManager::new(&client, 384).setup_all().await.is_err());
        let rows = client
            .execute(
                "SHOW INDEXES YIELD name WHERE name = 'conversation_session_idx' RETURN name",
                vec![],
            )
            .await
            .unwrap();
        assert!(
            rows.is_empty(),
            "rejected version must not change product DDL"
        );
    }
    client
        .execute(
            "MATCH (s:MuninnSchema) SET s.version = 0, s.state = 'ready'",
            vec![],
        )
        .await
        .unwrap();
    SchemaManager::new(&client, 384).setup_all().await.unwrap();

    // Upgrade must reject an existing incompatible named definition BEFORE
    // creating any of the still-missing managed objects.
    client
        .execute("DROP INDEX conversation_session_idx", vec![])
        .await
        .unwrap();
    client
        .execute("DROP INDEX message_timestamp_idx", vec![])
        .await
        .unwrap();
    client
        .execute(
            "CREATE RANGE INDEX message_timestamp_idx FOR (n:Message) ON (n.incompatible_fixture)",
            vec![],
        )
        .await
        .unwrap();
    let error = SchemaManager::new(&client, 384)
        .setup_all()
        .await
        .unwrap_err();
    assert!(error.to_string().contains("message_timestamp_idx"));
    let rows = client
        .execute(
            "SHOW INDEXES YIELD name WHERE name = 'conversation_session_idx' RETURN name",
            vec![],
        )
        .await
        .unwrap();
    assert!(
        rows.is_empty(),
        "incompatible adoption must not create missing managed indexes"
    );
    client
        .execute("DROP INDEX message_timestamp_idx", vec![])
        .await
        .unwrap();
    SchemaManager::new(&client, 384).setup_all().await.unwrap();
    let rows = client
        .execute(
            "MATCH (n:MigrationSentinel {value: 'preserved'}) RETURN count(n) AS count",
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(rows[0].get::<i64>("count").unwrap(), 1);
    // A non-vector index occupying a vector name must reject before any DDL.
    client
        .execute("DROP INDEX conversation_session_idx", vec![])
        .await
        .unwrap();
    client
        .execute("DROP INDEX message_embedding_idx", vec![])
        .await
        .unwrap();
    client
        .execute(
            "CREATE RANGE INDEX message_embedding_idx FOR (n:Message) ON (n.vector_collision)",
            vec![],
        )
        .await
        .unwrap();
    assert!(SchemaManager::new(&client, 384).setup_all().await.is_err());
    let rows = client
        .execute(
            "SHOW INDEXES YIELD name WHERE name = 'conversation_session_idx' RETURN name",
            vec![],
        )
        .await
        .unwrap();
    assert!(
        rows.is_empty(),
        "vector name collision must reject before product DDL"
    );
    client
        .execute("DROP INDEX message_embedding_idx", vec![])
        .await
        .unwrap();
    SchemaManager::new(&client, 384).setup_all().await.unwrap();
    client
        .execute("DROP INDEX entity_location_idx", vec![])
        .await
        .unwrap();
    client
        .execute("DROP INDEX conversation_session_idx", vec![])
        .await
        .unwrap();
    client
        .execute(
            "CREATE RANGE INDEX entity_location_idx FOR (n:Entity) ON (n.wrong_location)",
            vec![],
        )
        .await
        .unwrap();
    assert!(
        SchemaManager::new(&client, 384).setup_all().await.is_err(),
        "POINT collision must reject"
    );
    let rows = client
        .execute(
            "SHOW INDEXES YIELD name WHERE name = 'conversation_session_idx' RETURN name",
            vec![],
        )
        .await
        .unwrap();
    assert!(rows.is_empty());
    client
        .execute("DROP INDEX entity_location_idx", vec![])
        .await
        .unwrap();
    SchemaManager::new(&client, 384).setup_all().await.unwrap();
    client
        .execute("DROP CONSTRAINT conversation_id", vec![])
        .await
        .unwrap();
    client
        .execute("DROP CONSTRAINT message_id", vec![])
        .await
        .unwrap();
    client
        .execute(
            "CREATE RANGE INDEX message_id FOR (n:CollisionFixture) ON (n.id)",
            vec![],
        )
        .await
        .unwrap();
    assert!(SchemaManager::new(&client, 384).setup_all().await.is_err());
    let rows = client
        .execute(
            "SHOW CONSTRAINTS YIELD name WHERE name = 'conversation_id' RETURN name",
            vec![],
        )
        .await
        .unwrap();
    assert!(
        rows.is_empty(),
        "constraint name collision must reject before product DDL"
    );
    // Maintenance clears memories, not the migration coordination boundary.
    use muninn_core::ports::GraphMaintenance;
    muninn_neo4j::Neo4jMaintenance::new(client.clone())
        .clear_all_data()
        .await
        .unwrap();
    let rows = client
        .execute(
            "MATCH (s:MuninnSchema {id: 'product'}) RETURN count(s) AS count",
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(
        rows[0].get::<i64>("count").unwrap(),
        1,
        "memory reset must preserve schema metadata"
    );
    let rows = client
        .execute(
            "MATCH (n:MigrationSentinel) RETURN count(n) AS count",
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(rows[0].get::<i64>("count").unwrap(), 0);
}
