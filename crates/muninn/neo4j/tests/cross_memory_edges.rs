//! Live cross-memory (`ABOUT`) verification, never run against an existing
//! database. Ignored means NOT_RUN until a new authenticated empty synthetic
//! fixture exists; the caller owns teardown.
use muninn_application::{
    AddEntityOptions, AddFactOptions, AddPreferenceOptions, MemoryClient, MemoryOptions,
    MemoryPorts,
};
use muninn_core::ports::{Embedder, LongTermStore};
use muninn_core::MemoryResult;
use muninn_neo4j::{
    Neo4jClient, Neo4jConfig, Neo4jLongTermStore, Neo4jReasoningStore, Neo4jShortTermStore,
    SchemaManager,
};
use std::sync::Arc;
use uuid::Uuid;

async fn snapshot(client: &Neo4jClient) -> String {
    let rows = client
        .execute(
            "MATCH (n) WITH count(n) AS nodes OPTIONAL MATCH ()-[r]->() RETURN nodes, count(r) AS edges",
            vec![],
        )
        .await
        .unwrap();
    format!(
        "{}:{}",
        rows[0].get::<i64>("nodes").unwrap(),
        rows[0].get::<i64>("edges").unwrap()
    )
}

#[tokio::test]
#[ignore = "requires a NEW authenticated empty synthetic Neo4j; caller owns teardown"]
async fn preference_about_edge_shape_idempotency_and_owner_isolation() {
    assert_eq!(
        std::env::var("MUNINN_CROSS_MEMORY_FRESH_SYNTHETIC").as_deref(),
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
    let long_term = Arc::new(Neo4jLongTermStore::new(client.clone()));
    let memory = MemoryClient::new(
        MemoryPorts {
            short_term: Arc::new(Neo4jShortTermStore::new(client.clone())),
            long_term: long_term.clone(),
            reasoning: Arc::new(Neo4jReasoningStore::new(client.clone())),
            embedder: None,
            cypher_query: None,
        },
        &MemoryOptions::default(),
    )
    .unwrap();
    let entity = memory
        .long_term()
        .add_entity(
            "Alice",
            "PERSON",
            AddEntityOptions {
                generate_embedding: false,
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .0;
    let preference = memory
        .long_term()
        .add_preference(
            "food",
            "tea",
            AddPreferenceOptions {
                user_identifier: Some("owner-a".into()),
                about_entities: vec![entity.id],
                generate_embedding: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    // Shape and direction: exactly one property-free Preference->Entity ABOUT.
    let shape = "MATCH (p:Preference {id: $id})-[r:ABOUT]->(e:Entity) \
                 RETURN count(r) AS edges, collect(type(r)) AS kinds, \
                 collect(size(keys(r))) AS properties, collect(e.id) AS targets";
    let rows = client
        .execute(shape, vec![("id", preference.id.to_string().into())])
        .await
        .unwrap();
    assert_eq!(rows[0].get::<i64>("edges").unwrap(), 1);
    assert_eq!(rows[0].get::<Vec<String>>("kinds").unwrap(), vec!["ABOUT"]);
    assert_eq!(rows[0].get::<Vec<i64>>("properties").unwrap(), vec![0]);
    assert_eq!(
        rows[0].get::<Vec<String>>("targets").unwrap(),
        vec![entity.id.to_string()]
    );
    // Replaying the same link adds no edge and changes no property.
    let before = snapshot(&client).await;
    for _ in 0..2 {
        long_term
            .link_preference_about_entity(preference.id, entity.id)
            .await
            .unwrap();
    }
    assert_eq!(snapshot(&client).await, before);
    let rows = client
        .execute(
            "MATCH (p:Preference {id: $id})-[r:ABOUT]->(:Entity) RETURN count(r) AS edges",
            vec![("id", preference.id.to_string().into())],
        )
        .await
        .unwrap();
    assert_eq!(rows[0].get::<i64>("edges").unwrap(), 1);
    // An unknown target fails explicitly and writes nothing.
    let missing = Uuid::new_v4();
    let before = snapshot(&client).await;
    let error = memory
        .long_term()
        .add_preference(
            "food",
            "tea",
            AddPreferenceOptions {
                user_identifier: Some("owner-a".into()),
                about_entities: vec![missing],
                generate_embedding: false,
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), format!("not found: entity {missing}"));
    assert_eq!(snapshot(&client).await, before);
    // A cross-owner read attempt reaches neither the preference nor its edge.
    let other = "owner-b";
    assert!(long_term
        .preferences_for_filtered(other, true, None, None)
        .await
        .unwrap()
        .is_empty());
    for (owner, expected) in [("owner-b", 0_i64), ("owner-a", 1)] {
        let rows = client
            .execute(
                "MATCH (:User {identifier: $owner})-[:HAS_PREFERENCE]->(:Preference)\
                 -[:ABOUT]->(:Entity) RETURN count(*) AS edges",
                vec![("owner", owner.into())],
            )
            .await
            .unwrap();
        assert_eq!(rows[0].get::<i64>("edges").unwrap(), expected);
    }
}

struct FactEmbedder;

#[async_trait::async_trait]
impl Embedder for FactEmbedder {
    fn dimensions(&self) -> usize {
        2
    }

    async fn embed(&self, _: &str) -> MemoryResult<Vec<f64>> {
        Ok(vec![1.0, 0.0])
    }
}

#[tokio::test]
#[ignore = "requires a NEW authenticated empty synthetic Neo4j; caller owns teardown"]
async fn fact_about_edge_shape_idempotency_and_missing_entity_rejection() {
    assert_eq!(
        std::env::var("MUNINN_CROSS_MEMORY_FRESH_SYNTHETIC").as_deref(),
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
    client
        .execute("CALL db.awaitIndexes(60)", vec![])
        .await
        .unwrap();
    let long_term = Arc::new(Neo4jLongTermStore::new(client.clone()));
    let memory = MemoryClient::new(
        MemoryPorts {
            short_term: Arc::new(Neo4jShortTermStore::new(client.clone())),
            long_term: long_term.clone(),
            reasoning: Arc::new(Neo4jReasoningStore::new(client.clone())),
            embedder: Some(Arc::new(FactEmbedder)),
            cypher_query: None,
        },
        &MemoryOptions::default(),
    )
    .unwrap();
    // Create an entity to link the fact to.
    let entity = memory
        .long_term()
        .add_entity(
            "Paris",
            "LOCATION",
            AddEntityOptions {
                generate_embedding: false,
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .0;
    let second = memory
        .long_term()
        .add_entity(
            "France",
            "LOCATION",
            AddEntityOptions {
                generate_embedding: false,
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .0;
    let unlinked = memory
        .long_term()
        .add_entity(
            "Europe",
            "LOCATION",
            AddEntityOptions {
                generate_embedding: false,
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .0;
    // Add a fact with about_entities.
    let fact = memory
        .long_term()
        .add_fact(
            "Paris",
            "is capital of",
            "France",
            AddFactOptions {
                about_entities: vec![entity.id, entity.id],
                confidence: 0.5,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    // Shape and direction: exactly one property-free Fact->Entity ABOUT.
    let shape = "MATCH (f:Fact {id: $id})-[r:ABOUT]->(e:Entity) \
                 RETURN count(r) AS edges, collect(type(r)) AS kinds, \
                 collect(size(keys(r))) AS properties, collect(e.id) AS targets";
    let rows = client
        .execute(shape, vec![("id", fact.id.to_string().into())])
        .await
        .unwrap();
    assert_eq!(rows[0].get::<i64>("edges").unwrap(), 1);
    assert_eq!(rows[0].get::<Vec<String>>("kinds").unwrap(), vec!["ABOUT"]);
    assert_eq!(rows[0].get::<Vec<i64>>("properties").unwrap(), vec![0]);
    assert_eq!(
        rows[0].get::<Vec<String>>("targets").unwrap(),
        vec![entity.id.to_string()]
    );
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    while long_term
        .find_duplicate_fact(&[1.0, 0.0], "Paris", "is capital of", 0.9)
        .await
        .unwrap()
        .is_none()
    {
        assert!(
            tokio::time::Instant::now() < deadline,
            "fact index did not converge"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    // A duplicate adds the new target without losing the existing target.
    // Replaying that request does not create a new fact or edge.
    let mut after_first = None;
    for _ in 0..2 {
        let duplicate = memory
            .long_term()
            .add_fact(
                "Paris",
                "is capital of",
                "French Republic",
                AddFactOptions {
                    about_entities: vec![second.id, second.id],
                    confidence: 0.8,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(duplicate.id, fact.id);
        assert_eq!(duplicate.metadata["deduplicated"], true);
        let current = snapshot(&client).await;
        if let Some(previous) = &after_first {
            assert_eq!(&current, previous);
        }
        after_first = Some(current);
    }
    let rows = client
        .execute(shape, vec![("id", fact.id.to_string().into())])
        .await
        .unwrap();
    assert_eq!(rows[0].get::<i64>("edges").unwrap(), 2);
    assert_eq!(rows[0].get::<Vec<i64>>("properties").unwrap(), vec![0, 0]);
    let mut targets = rows[0].get::<Vec<String>>("targets").unwrap();
    targets.sort_unstable();
    let mut expected = [entity.id.to_string(), second.id.to_string()];
    expected.sort_unstable();
    assert_eq!(targets, expected);
    let stored = long_term.facts_by_subject("Paris", 10).await.unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].confidence, 0.8);
    assert_eq!(stored[0].object, "French Republic");
    // An unknown target fails explicitly and writes no fact.
    let missing = Uuid::new_v4();
    let before = snapshot(&client).await;
    let error = memory
        .long_term()
        .add_fact(
            "London",
            "is capital of",
            "UK",
            AddFactOptions {
                about_entities: vec![unlinked.id, missing],
                generate_embedding: false,
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), format!("not found: entity {missing}"));
    assert_eq!(snapshot(&client).await, before);
    let error = memory
        .long_term()
        .add_fact(
            "Paris",
            "is capital of",
            "Should not persist",
            AddFactOptions {
                about_entities: vec![unlinked.id, missing],
                confidence: 1.0,
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), format!("not found: entity {missing}"));
    assert_eq!(snapshot(&client).await, before);
    assert_eq!(
        long_term.facts_by_subject("Paris", 10).await.unwrap(),
        stored
    );
}
