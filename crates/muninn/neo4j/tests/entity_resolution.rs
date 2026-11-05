//! Live graph effects, opt-in only. An ignored test is NOT_RUN, not acceptance.
use std::sync::Arc;

use async_trait::async_trait;
use muninn_application::{AddEntityOptions, LongTermMemory};
use muninn_core::ports::{Embedder, EntityUpdate, LongTermStore};
use muninn_core::resolution::{DeduplicationAction, ResolutionConfig};
use muninn_core::MemoryResult;
use muninn_neo4j::{Neo4jClient, Neo4jConfig, Neo4jLongTermStore, SchemaManager};
use serde_json::Value;

/// Test-only controlled vector; this is not MiniLM or a quality measurement.
struct TestEmbedder;
#[async_trait]
impl Embedder for TestEmbedder {
    fn dimensions(&self) -> usize {
        2
    }
    async fn embed(&self, _: &str) -> MemoryResult<Vec<f64>> {
        Ok(vec![1.0, 0.0])
    }
}

async fn count(client: &Neo4jClient, cypher: &str) -> i64 {
    client.execute(cypher, vec![]).await.unwrap()[0]
        .get("count")
        .unwrap()
}

#[tokio::test]
#[ignore = "requires a NEW authenticated empty synthetic Neo4j; caller owns fixture teardown"]
async fn entity_resolution_pending_merge_distinct_and_rejection_effects() {
    assert_eq!(
        std::env::var("MUNINN_RESOLUTION_FRESH_SYNTHETIC").as_deref(),
        Ok("1")
    );
    let config = Neo4jConfig {
        uri: std::env::var("MUNINN_FIXTURE_URI").expect("fixture URI required"),
        username: "neo4j".into(),
        password: std::env::var("MUNINN_FIXTURE_PASSWORD").expect("fixture password required"),
        ..Default::default()
    };
    let client = Neo4jClient::connect(&config).await.unwrap();
    assert_eq!(
        count(&client, "MATCH (n) RETURN count(n) AS count").await,
        0
    );
    SchemaManager::new(&client, 2).setup_all().await.unwrap();
    let store = Arc::new(Neo4jLongTermStore::new(client.clone()));
    let memory = LongTermMemory::new(store.clone(), Some(Arc::new(TestEmbedder)), false);
    let (candidate, no_resolution) = memory
        .add_entity(
            "zzzzzzzzzzzzzzzzzzzz",
            "PERSON",
            AddEntityOptions {
                generate_embedding: false,
                description: Some("untouched candidate".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(no_resolution.action, DeduplicationAction::None);
    // Neo4j cosine scores are normalized: cosine 0.74 becomes score 0.87.
    store
        .update_entity(
            candidate.id,
            &EntityUpdate {
                embedding: Some(vec![0.74, 0.74_f64.mul_add(-0.74, 1.0).sqrt()]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let before: Value =
        serde_json::to_value(memory.get_entity(candidate.id).await.unwrap().unwrap()).unwrap();
    let options = || AddEntityOptions {
        resolution: Some(ResolutionConfig::default()),
        ..Default::default()
    };
    let (incoming, flagged) = memory
        .add_entity("abcdefghijklmnopqrst", "PERSON", options())
        .await
        .unwrap();
    assert_eq!(flagged.action, DeduplicationAction::Flagged);
    assert_eq!(flagged.matched_entity_id, Some(candidate.id));
    assert_ne!(incoming.id, candidate.id);
    assert_eq!(
        count(&client, "MATCH (:Entity) RETURN count(*) AS count").await,
        2
    );
    assert_eq!(
        count(
            &client,
            "MATCH ()-[r:SAME_AS {status:'pending'}]->() RETURN count(r) AS count"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &client,
            "MATCH (e:Entity) WHERE e.merged_into IS NOT NULL RETURN count(e) AS count"
        )
        .await,
        0
    );
    assert_eq!(
        serde_json::to_value(memory.get_entity(candidate.id).await.unwrap().unwrap()).unwrap(),
        before
    );
    let (repeat, _) = memory
        .add_entity("abcdefghijklmnopqrst", "PERSON", options())
        .await
        .unwrap();
    assert_eq!(repeat.id, incoming.id);
    assert_eq!(
        count(&client, "MATCH (:Entity) RETURN count(*) AS count").await,
        2
    );
    assert_eq!(
        count(&client, "MATCH ()-[r:SAME_AS]->() RETURN count(r) AS count").await,
        1
    );

    let (merged, result) = memory
        .add_entity("Mary Jane", "PERSON", options())
        .await
        .unwrap();
    // Incoming is the sole score-1 vector candidate, so no throwaway node is created.
    assert_eq!(result.action, DeduplicationAction::Merged);
    assert_eq!(merged.id, incoming.id);
    assert!(merged.aliases.contains(&"Mary Jane".into()));
    assert_eq!(
        count(&client, "MATCH (:Entity) RETURN count(*) AS count").await,
        2
    );
    let (_, distinct) = memory
        .add_entity("independent", "OBJECT", options())
        .await
        .unwrap();
    assert_eq!(distinct.action, DeduplicationAction::None);
    assert_eq!(
        count(&client, "MATCH (:Entity) RETURN count(*) AS count").await,
        3
    );
    assert_eq!(
        count(&client, "MATCH ()-[r:SAME_AS]->() RETURN count(r) AS count").await,
        1
    );

    let snapshot = memory.list_entities(None, 100).await.unwrap();
    let error = memory
        .add_entity(
            "rejected",
            "PERSON",
            AddEntityOptions {
                resolution: Some(ResolutionConfig {
                    review_threshold: 0.91,
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "invalid input: resolution.review_threshold must be <= resolution.auto_merge_threshold (got 0.91 > 0.9)");
    assert_eq!(
        serde_json::to_value(memory.list_entities(None, 100).await.unwrap()).unwrap(),
        serde_json::to_value(snapshot).unwrap()
    );
    assert_eq!(
        count(&client, "MATCH ()-[r]->() RETURN count(r) AS count").await,
        1
    );
    assert_eq!(
        count(
            &client,
            "MATCH (e:Entity) WHERE e.merged_into IS NOT NULL RETURN count(e) AS count"
        )
        .await,
        0
    );
}
