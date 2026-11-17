//! Run only against an explicitly configured, fresh synthetic Neo4j fixture.
//!
//! H2 dispositions for the long-term surface: user isolation on the preference
//! reads, and the deletion semantics of the graph (`DETACH DELETE` removes the
//! entity and its edges while independent records survive).
use chrono::Utc;
use muninn_core::model::{Entity, Metadata, Preference};
use muninn_core::ports::LongTermStore;
use muninn_neo4j::{Neo4jClient, Neo4jConfig, Neo4jLongTermStore};
use uuid::Uuid;

fn entity(name: String) -> Entity {
    Entity {
        id: Uuid::new_v4(),
        name,
        canonical_name: None,
        entity_type: "OBJECT".into(),
        subtype: None,
        description: None,
        confidence: 1.0,
        aliases: vec![],
        embedding: None,
        created_at: Utc::now(),
        updated_at: None,
        metadata: Metadata::new(),
    }
}

fn preference(category: &str, text: &str) -> Preference {
    Preference {
        id: Uuid::new_v4(),
        category: category.to_owned(),
        preference: text.to_owned(),
        context: None,
        confidence: 1.0,
        embedding: None,
        valid_from: None,
        valid_until: None,
        created_at: Utc::now(),
        metadata: Metadata::new(),
    }
}

async fn owned(store: &Neo4jLongTermStore, user: &str, target: Option<Uuid>) -> Vec<Uuid> {
    store
        .preferences_for_filtered(user, false, target, None)
        .await
        .unwrap()
        .iter()
        .map(|item| item.id)
        .collect()
}

async fn relationships_between(client: &Neo4jClient, source: Uuid, target: Uuid) -> i64 {
    client
        .execute(
            "MATCH (a:Entity {id: $source})-[r]->(b:Entity {id: $target}) RETURN r",
            vec![
                ("source", source.to_string().into()),
                ("target", target.to_string().into()),
            ],
        )
        .await
        .unwrap()
        .len() as i64
}

#[tokio::test]
#[ignore = "requires explicitly configured fresh synthetic Neo4j"]
async fn long_term_user_isolation_and_entity_deletion_semantics() {
    let client = Neo4jClient::connect(&Neo4jConfig {
        uri: std::env::var("MUNINN_FIXTURE_URI").unwrap(),
        username: "neo4j".into(),
        password: std::env::var("MUNINN_FIXTURE_PASSWORD").unwrap(),
        ..Neo4jConfig::default()
    })
    .await
    .unwrap();
    let store = Neo4jLongTermStore::new(client.clone());
    let alice = format!("isolation-alice-{}", Uuid::new_v4());
    let bob = format!("isolation-bob-{}", Uuid::new_v4());

    // Two entities, one of them targeted by both users' preferences.
    let shared = entity(format!("shared-{}", Uuid::new_v4()));
    let peer = entity(format!("peer-{}", Uuid::new_v4()));
    store.upsert_entity(&shared).await.unwrap();
    store.upsert_entity(&peer).await.unwrap();
    client
        .execute(
            "MATCH (a:Entity {id: $a}), (b:Entity {id: $b}) CREATE (a)-[:RELATED_TO {id: $r}]->(b)",
            vec![
                ("a", shared.id.to_string().into()),
                ("b", peer.id.to_string().into()),
                ("r", Uuid::new_v4().to_string().into()),
            ],
        )
        .await
        .unwrap();
    assert_eq!(relationships_between(&client, shared.id, peer.id).await, 1);

    // Same target, different owners: the two preferences must never mix.
    let alice_preference = preference(&alice, "prefiere te");
    let bob_preference = preference(&bob, "prefiere cafe");
    for (user, item) in [(&alice, &alice_preference), (&bob, &bob_preference)] {
        store.create_preference(item).await.unwrap();
        store.link_user_to_preference(user, item.id).await.unwrap();
        store
            .link_preference_to_entity(item.id, shared.id)
            .await
            .unwrap();
    }

    // Isolation: each owner reads only its own record, with or without target filter.
    for (user, expected, foreign) in [
        (&alice, alice_preference.id, bob_preference.id),
        (&bob, bob_preference.id, alice_preference.id),
    ] {
        let plain: Vec<Uuid> = store
            .preferences_for(user, false)
            .await
            .unwrap()
            .iter()
            .map(|item| item.id)
            .collect();
        assert_eq!(plain, vec![expected]);
        assert_eq!(owned(&store, user, Some(shared.id)).await, vec![expected]);
        assert!(!plain.contains(&foreign));
        assert!(owned(&store, "isolation-unknown-user", Some(shared.id))
            .await
            .is_empty());
        assert!(store
            .preferences_for("isolation-unknown-user", false)
            .await
            .unwrap()
            .is_empty());
    }

    // Deleting an unrelated entity disturbs nothing.
    assert!(store.delete_entity(peer.id).await.unwrap());
    assert!(!store.delete_entity(peer.id).await.unwrap());
    assert!(store.get_entity(peer.id).await.unwrap().is_none());
    assert_eq!(
        owned(&store, &alice, Some(shared.id)).await,
        vec![alice_preference.id]
    );

    // Deleting the targeted entity removes the node and its edges, and detaches
    // every target filter, while both preference records and their owners survive.
    assert!(store.delete_entity(shared.id).await.unwrap());
    assert!(store.get_entity(shared.id).await.unwrap().is_none());
    assert_eq!(relationships_between(&client, shared.id, peer.id).await, 0);
    assert!(owned(&store, &alice, Some(shared.id)).await.is_empty());
    assert!(owned(&store, &bob, Some(shared.id)).await.is_empty());
    assert_eq!(
        store
            .preferences_for(&alice, false)
            .await
            .unwrap()
            .iter()
            .map(|item| item.id)
            .collect::<Vec<_>>(),
        vec![alice_preference.id]
    );
    assert_eq!(
        store
            .preferences_for(&bob, false)
            .await
            .unwrap()
            .iter()
            .map(|item| item.id)
            .collect::<Vec<_>>(),
        vec![bob_preference.id]
    );
    assert!(store
        .related_entities(shared.id, 1)
        .await
        .unwrap()
        .is_empty());
}
