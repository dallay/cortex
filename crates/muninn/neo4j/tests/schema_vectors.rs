//! Opt-in checks against a NEW disposable Neo4j 5.26 fixture only.
//! Never point these variables at an existing application database.
use muninn_neo4j::{Neo4jClient, Neo4jConfig, SchemaManager};

async fn fixture() -> Neo4jClient {
    assert_eq!(
        std::env::var("MUNINN_SCHEMA_ISOLATED_TEST").as_deref(),
        Ok("1"),
        "requires explicit opt-in for a NEW disposable fixture"
    );
    Neo4jClient::connect(&Neo4jConfig {
        uri: std::env::var("MUNINN_SCHEMA_TEST_URI").expect("fixture URI required"),
        username: "neo4j".into(),
        password: std::env::var("MUNINN_SCHEMA_TEST_PASSWORD").expect("fixture password required"),
        ..Default::default()
    })
    .await
    .expect("connect to disposable fixture")
}

#[tokio::test]
#[ignore = "requires explicit opt-in and a NEW disposable Neo4j 5.26 fixture"]
async fn vector_schema_is_idempotent_and_rejects_wrong_dimensions() {
    let client = fixture().await;
    let schema = SchemaManager::new(&client, 3);
    schema.setup_all().await.expect("create required schema");
    schema.setup_all().await.expect("idempotent setup");
    let inventory = schema.vector_index_inventory().await.unwrap();
    assert_eq!(inventory.len(), 6);
    for index in &inventory {
        assert_eq!(index["state"], "ONLINE");
        assert_eq!(index["options"]["indexConfig"]["vector.dimensions"], 3);
        assert_eq!(
            index["options"]["indexConfig"]["vector.similarity_function"],
            "COSINE"
        );
    }
    println!("VECTOR inventory: {inventory:#?}");
    // IF NOT EXISTS must not mask a different embedding dimension.
    let incompatible = SchemaManager::new(&client, 4);
    let error = incompatible.setup_all().await.unwrap_err().to_string();
    assert!(error.contains("message_embedding"), "{error}");
    assert!(error.contains("vector.dimensions"), "{error}");
    assert!(error.contains('4'), "{error}");
    // A rejected setup must preserve the existing indexes.
    assert_eq!(inventory, schema.vector_index_inventory().await.unwrap());
}

#[tokio::test]
#[ignore = "requires explicit opt-in and a NEW disposable Neo4j 5.26 fixture"]
async fn concurrent_schema_setup_requires_equivalent_schema() {
    let client = fixture().await;
    let mut tasks = tokio::task::JoinSet::new();
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(12));
    for _ in 0..12 {
        let client = client.clone();
        let barrier = barrier.clone();
        tasks.spawn(async move {
            barrier.wait().await;
            SchemaManager::new(&client, 3).setup_all().await
        });
    }
    let mut failures = Vec::new();
    while let Some(result) = tasks.join_next().await {
        if let Err(error) = result.unwrap() {
            failures.push(error.to_string());
        }
    }
    assert!(
        failures.is_empty(),
        "concurrent setup failed: {failures:#?}"
    );
    SchemaManager::new(&client, 3)
        .validate_vector_index_dimensions(3)
        .await
        .unwrap();
    assert!(SchemaManager::new(&client, 4).setup_all().await.is_err());
}
