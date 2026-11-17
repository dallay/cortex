//! Observation persistence contract against a fresh disposable Neo4j fixture.
//! The caller owns fixture setup and teardown; this test never clears data.
use chrono::{TimeZone, Utc};
use muninn_core::model::Observation;
use muninn_core::ports::{OwnerScope, ShortTermStore};
use muninn_neo4j::{Neo4jClient, Neo4jConfig, Neo4jShortTermStore, SchemaManager};
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires a NEW empty disposable authenticated Neo4j fixture"]
async fn observations_persist_append_scope_and_paginate_deterministically() {
    assert_eq!(
        std::env::var("MUNINN_OBSERVATION_ISOLATED_TEST").as_deref(),
        Ok("1")
    );
    let client = Neo4jClient::connect(&Neo4jConfig {
        uri: std::env::var("MUNINN_OBSERVATION_TEST_URI").expect("fixture URI required"),
        username: "neo4j".into(),
        password: std::env::var("MUNINN_OBSERVATION_TEST_PASSWORD")
            .expect("fixture password required"),
        ..Neo4jConfig::default()
    })
    .await
    .expect("connect to disposable fixture");
    let rows = client
        .execute("MATCH (n) RETURN count(n) AS count", vec![])
        .await
        .unwrap();
    assert_eq!(
        rows[0].get::<i64>("count").unwrap(),
        0,
        "fixture must be empty"
    );

    SchemaManager::new(&client, 3)
        .setup_all()
        .await
        .expect("schema setup");
    let store = Neo4jShortTermStore::new(client.clone());
    let owner_a = OwnerScope::owned(&format!("observation-a-{}", Uuid::new_v4())).unwrap();
    let owner_b = OwnerScope::owned(&format!("observation-b-{}", Uuid::new_v4())).unwrap();
    let session = Uuid::new_v4().to_string();
    let conversation_id = store
        .ensure_conversation(&session, None, &owner_a)
        .await
        .unwrap();

    assert!(store
        .list_observations(conversation_id, &owner_a, 100, 0)
        .await
        .unwrap()
        .is_empty());
    let timestamp = Utc.with_ymd_and_hms(2026, 11, 15, 10, 0, 0).unwrap();
    let first = Observation {
        id: Uuid::new_v4(),
        conversation_id,
        content: "first".into(),
        created_at: timestamp,
    };
    let second = Observation {
        id: Uuid::new_v4(),
        conversation_id,
        content: "second".into(),
        created_at: timestamp,
    };
    store.append_observation(&first, &owner_a).await.unwrap();
    store.append_observation(&second, &owner_a).await.unwrap();

    let mut expected = vec![first.clone(), second.clone()];
    expected.sort_by_key(|item| (item.created_at, item.id));
    assert_eq!(
        store
            .list_observations(conversation_id, &owner_a, 100, 0)
            .await
            .unwrap(),
        expected,
        "same timestamp rows must have stable UUID ordering"
    );
    assert_eq!(
        store
            .list_observations(conversation_id, &owner_a, 1, 1)
            .await
            .unwrap(),
        vec![expected[1].clone()]
    );

    let other_conversation = store
        .ensure_conversation(&Uuid::new_v4().to_string(), None, &owner_b)
        .await
        .unwrap();
    assert!(store
        .list_observations(conversation_id, &owner_b, 100, 0)
        .await
        .is_err());
    assert!(store
        .append_observation(
            &Observation {
                id: Uuid::new_v4(),
                conversation_id,
                content: "foreign write".into(),
                created_at: Utc::now(),
            },
            &owner_b,
        )
        .await
        .is_err());
    assert!(store
        .list_observations(Uuid::new_v4(), &owner_a, 100, 0)
        .await
        .is_err());
    assert!(store
        .append_observation(
            &Observation {
                id: Uuid::new_v4(),
                conversation_id: Uuid::new_v4(),
                content: "orphan".into(),
                created_at: Utc::now(),
            },
            &owner_a,
        )
        .await
        .is_err());
    assert!(store
        .list_observations(other_conversation, &owner_b, 100, 0)
        .await
        .unwrap()
        .is_empty());
}
