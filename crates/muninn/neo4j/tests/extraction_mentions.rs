//! Opt-in live graph verification, never run against an existing database.
use async_trait::async_trait;
use muninn_application::{AddMessageOptions, MemoryClient, MemoryOptions, MemoryPorts};
use muninn_core::extraction::{EntityExtractor, ExtractOptions, ExtractionMode, ExtractionResult};
use muninn_core::model::{EntityRef, MessageRole};
use muninn_core::{MemoryError, MemoryResult};
use muninn_neo4j::{
    Neo4jClient, Neo4jConfig, Neo4jLongTermStore, Neo4jReasoningStore, Neo4jShortTermStore,
    SchemaManager,
};
use std::sync::Arc;

struct FailingStage;
#[async_trait]
impl EntityExtractor for FailingStage {
    async fn extract(&self, _: &str, _: &ExtractOptions) -> MemoryResult<ExtractionResult> {
        Err(MemoryError::provider("synthetic unavailable"))
    }
}

async fn snapshot(client: &Neo4jClient) -> String {
    let rows = client.execute("MATCH (n) WITH count(n) AS nodes OPTIONAL MATCH ()-[r]->() RETURN nodes, count(r) AS edges", vec![]).await.unwrap();
    format!(
        "{}:{}",
        rows[0].get::<i64>("nodes").unwrap(),
        rows[0].get::<i64>("edges").unwrap()
    )
}

#[tokio::test]
#[ignore = "requires a NEW authenticated empty synthetic Neo4j; caller owns teardown"]
async fn extraction_mentions_provenance_idempotency_and_failure_atomicity() {
    assert_eq!(
        std::env::var("MUNINN_EXTRACTION_FRESH_SYNTHETIC").as_deref(),
        Ok("1")
    );
    let client = Neo4jClient::connect(&Neo4jConfig {
        uri: std::env::var("MUNINN_FIXTURE_URI").expect("fixture URI required"),
        username: "neo4j".into(),
        password: std::env::var("MUNINN_FIXTURE_PASSWORD").expect("fixture password required"),
        ..Default::default()
    })
    .await
    .unwrap();
    assert_eq!(snapshot(&client).await, "0:0");
    SchemaManager::new(&client, 2).setup_all().await.unwrap();
    let memory = MemoryClient::new(
        MemoryPorts {
            short_term: Arc::new(Neo4jShortTermStore::new(client.clone())),
            long_term: Arc::new(Neo4jLongTermStore::new(client.clone())),
            reasoning: Arc::new(Neo4jReasoningStore::new(client.clone())),
            embedder: None,
            cypher_query: None,
        },
        &MemoryOptions::default(),
    )
    .with_extractor(Arc::new(FailingStage));
    let before = snapshot(&client).await;
    let error = memory
        .short_term()
        .add_message(
            "failure",
            MessageRole::User,
            "Alice",
            AddMessageOptions {
                extraction_mode: Some(ExtractionMode::Auto),
                user_identifier: Some("owner".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "provider error: synthetic unavailable");
    assert_eq!(snapshot(&client).await, before);
    let options = || AddMessageOptions {
        extraction_mode: Some(ExtractionMode::Explicit),
        operation_key: Some("once".into()),
        explicit_mentions: vec![EntityRef::NameType {
            name: "Alice".into(),
            entity_type: "PERSON".into(),
        }],
        generate_embedding: false,
        user_identifier: Some("owner".into()),
        ..Default::default()
    };
    let message = memory
        .short_term()
        .add_message("fixture", MessageRole::User, "Alice", options())
        .await
        .unwrap();
    let before = snapshot(&client).await;
    let replay = memory
        .short_term()
        .add_message("fixture", MessageRole::User, "Alice", options())
        .await
        .unwrap();
    assert_eq!(replay.id, message.id);
    assert_eq!(snapshot(&client).await, before);
    let rows = client.execute("MATCH (c:Conversation)-[:HAS_MESSAGE]->(m:Message)-[r:MENTIONS]->(e:Entity) RETURN count(r) AS count, collect(r.derived) AS derived, collect(r.support) AS support, collect(r.source_message_ids) AS sources, collect(r.evidence) AS evidence",vec![]).await.unwrap();
    assert_eq!(rows[0].get::<i64>("count").unwrap(), 1);
    assert_eq!(rows[0].get::<Vec<bool>>("derived").unwrap(), vec![false]);
    assert_eq!(rows[0].get::<Vec<i64>>("support").unwrap(), vec![1]);
    assert_eq!(
        rows[0].get::<Vec<Vec<String>>>("sources").unwrap(),
        vec![vec![message.id.to_string()]]
    );
    assert_eq!(
        rows[0].get::<Vec<String>>("evidence").unwrap(),
        vec!["Alice"]
    );
    assert!(memory
        .short_term()
        .get_conversation("fixture", None, Some("other"), None)
        .await
        .unwrap()
        .messages
        .is_empty());
}
