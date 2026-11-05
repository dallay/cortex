//! Live cross-memory (`ABOUT`) verification, never run against an existing
//! database. Ignored means NOT_RUN until a new authenticated empty synthetic
//! fixture exists; the caller owns teardown.
use muninn_application::{
    AddEntityOptions, AddPreferenceOptions, MemoryClient, MemoryOptions, MemoryPorts,
};
use muninn_core::ports::LongTermStore;
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
    );
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
