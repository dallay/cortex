//! Requires a NEW, empty, disposable fixture. Never target application data.
use muninn_neo4j::{Neo4jClient, Neo4jConfig, SchemaManager};
use std::time::Duration;

async fn wait_for_blocked_query(client: &Neo4jClient, fragment: &str) {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let rows = client
                .execute(
                    "SHOW TRANSACTIONS YIELD currentQuery, status \
                     WHERE currentQuery CONTAINS $fragment AND status STARTS WITH 'Blocked' \
                     RETURN currentQuery",
                    vec![("fragment", fragment.into())],
                )
                .await
                .unwrap();
            if !rows.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("did not observe a blocked query containing {fragment}"));
}

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
    client
        .execute(
            "CREATE CONSTRAINT muninn_schema_id FOR (s:MuninnSchema) REQUIRE s.id IS UNIQUE",
            vec![],
        )
        .await
        .unwrap();
    client
        .execute(
            "CREATE (:MuninnSchema {id: 'product', version: 3, state: 'ready', revision: 0})",
            vec![],
        )
        .await
        .unwrap();
    async fn schema_names(client: &Neo4jClient) -> Vec<String> {
        let mut names = Vec::new();
        for query in ["SHOW INDEXES YIELD name", "SHOW CONSTRAINTS YIELD name"] {
            names.extend(
                client
                    .execute(query, vec![])
                    .await
                    .unwrap()
                    .into_iter()
                    .map(|row| row.get::<String>("name").unwrap()),
            );
        }
        names.sort_unstable();
        names
    }

    let before = schema_names(&client).await;
    let schema = SchemaManager::new(&client, 384);
    let error = schema.setup_all().await.unwrap_err().to_string();
    assert!(
        error.contains("incompatible Muninn schema version"),
        "{error}"
    );
    assert_eq!(schema_names(&client).await, before);
    client
        .execute("MATCH (s:MuninnSchema) SET s.version = 0", vec![])
        .await
        .unwrap();

    // Lock a product label with an uncommitted schema transaction
    // to pause the first manager inside DDL.
    let graph = neo4rs::Graph::new(&config.uri, &config.username, &config.password)
        .await
        .unwrap();
    let mut blocker = graph.start_txn().await.unwrap();
    blocker
        .run(neo4rs::query(
            "CREATE RANGE INDEX schema_gate_fixture FOR (n:ReasoningStep) ON (n.fixture)",
        ))
        .await
        .unwrap();
    let observer = Neo4jClient::connect(&config).await.unwrap();
    let competitor_client = Neo4jClient::connect(&config).await.unwrap();
    let competitor = SchemaManager::new(&competitor_client, 385);
    let (winner, (loser, ())) = tokio::join!(schema.setup_all(), async {
        wait_for_blocked_query(&observer, "CREATE CONSTRAINT reasoning_step_id").await;
        let rows = observer
            .execute("MATCH (s:MuninnSchema) RETURN s.version AS version", vec![])
            .await
            .unwrap();
        assert_eq!(
            rows[0].get::<i64>("version").unwrap(),
            0,
            "DDL is unfinished"
        );
        tokio::join!(competitor.setup_all(), async {
            // The competing manager must wait before running product DDL.
            wait_for_blocked_query(&observer, "MuninnSchema").await;
            blocker.rollback().await.unwrap();
        })
    });
    winner.unwrap();
    let error = loser.unwrap_err().to_string();
    assert!(error.contains("vector.dimensions"), "{error}");
    schema.validate_vector_index_dimensions(384).await.unwrap();
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
    assert_eq!(rows[0].get::<i64>("version").unwrap(), 2);
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
    assert_eq!(rows[0].get::<i64>("revision").unwrap(), 3);

    // A future or incomplete marker must reject before filling missing DDL.
    client
        .execute("DROP INDEX conversation_session_idx", vec![])
        .await
        .unwrap();
    for mutation in [
        "MATCH (s:MuninnSchema) SET s.version = 3",
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
