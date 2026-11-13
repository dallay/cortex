//! Characterization of current global graph semantics, not tenant isolation.
//! Run explicitly against a fresh synthetic Neo4j fixture only.
use chrono::Utc;
use muninn_core::model::{Entity, Fact, Metadata, Preference, Relationship};
use muninn_core::ports::{
    EmbeddingIdentity, LongTermStore, MessageRecord, OwnerScope, ShortTermStore,
};
use muninn_neo4j::{
    Neo4jClient, Neo4jConfig, Neo4jLongTermStore, Neo4jShortTermStore, SchemaManager,
};
use uuid::Uuid;

fn entity(name: &str) -> Entity {
    Entity {
        id: Uuid::new_v4(),
        name: name.into(),
        canonical_name: None,
        entity_type: "PERSON".into(),
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

fn preference(text: &str, category: &str) -> Preference {
    Preference {
        id: Uuid::new_v4(),
        category: category.into(),
        preference: text.into(),
        context: None,
        confidence: 1.0,
        embedding: Some(vec![1.0, 0.0, 0.0]),
        valid_from: None,
        valid_until: None,
        created_at: Utc::now(),
        metadata: Metadata::new(),
    }
}

async fn count(client: &Neo4jClient, cypher: &str, id: Uuid) -> i64 {
    client
        .execute(cypher, vec![("id", id.to_string().into())])
        .await
        .unwrap()[0]
        .get("n")
        .unwrap()
}

#[tokio::test]
#[ignore = "requires explicitly configured fresh synthetic Neo4j"]
async fn long_term_global_nodes_scoped_preferences_and_session_lifecycle() {
    let client = Neo4jClient::connect(&Neo4jConfig {
        uri: std::env::var("MUNINN_FIXTURE_URI").unwrap(),
        username: "neo4j".into(),
        password: std::env::var("MUNINN_FIXTURE_PASSWORD").unwrap(),
        ..Neo4jConfig::default()
    })
    .await
    .unwrap();
    SchemaManager::new(&client, 3).setup_all().await.unwrap();
    client
        .execute("CALL db.awaitIndexes(60)", vec![])
        .await
        .unwrap();
    let long = Neo4jLongTermStore::new(client.clone());
    let short = Neo4jShortTermStore::new(client.clone());
    let a = format!("lifecycle-a-{}", Uuid::new_v4());
    let b = format!("lifecycle-b-{}", Uuid::new_v4());
    let owner_a = OwnerScope::owned(&a).unwrap();
    let owner_b = OwnerScope::owned(&b).unwrap();
    let category = format!("lifecycle-{}", Uuid::new_v4());
    let pa = preference("A preference", &category);
    let pb = preference("B preference", &category);
    long.create_preference(&pa).await.unwrap();
    long.create_preference(&pb).await.unwrap();
    long.link_user_to_preference(&a, pa.id).await.unwrap();
    long.link_user_to_preference(&b, pb.id).await.unwrap();
    // Identical vectors in the same category: dedup must follow HAS_PREFERENCE.
    // Vector index updates are asynchronous; wait with a bounded deadline.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let hits = long
            .search_preferences(&[1.0, 0.0, 0.0], 100, 0.9, None)
            .await
            .unwrap();
        if [pa.id, pb.id]
            .iter()
            .all(|id| hits.iter().any(|hit| hit.item.id == *id))
        {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "preference index did not converge"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    for (user, expected) in [(&a, pa.id), (&b, pb.id)] {
        assert_eq!(
            long.preferences_for(user, true)
                .await
                .unwrap()
                .iter()
                .map(|p| p.id)
                .collect::<Vec<_>>(),
            vec![expected]
        );
        assert_eq!(
            long.preferences_by_category(&category, 100, Some(user))
                .await
                .unwrap()
                .iter()
                .map(|p| p.id)
                .collect::<Vec<_>>(),
            vec![expected]
        );
        assert_eq!(
            long.search_preferences(&[1.0, 0.0, 0.0], 100, 0.9, Some(user))
                .await
                .unwrap()
                .iter()
                .map(|p| p.item.id)
                .collect::<Vec<_>>(),
            vec![expected]
        );
        assert_eq!(
            long.find_duplicate_preference(&[1.0, 0.0, 0.0], &category, 0.9, Some(user))
                .await
                .unwrap()
                .unwrap()
                .id,
            expected
        );
    }
    assert_eq!(
        long.preferences_by_category(&category, 100, None)
            .await
            .unwrap()
            .len(),
        2
    );

    let shared = entity("shared");
    let neighbor = entity("neighbor");
    let survivor = entity("survivor");
    for e in [&shared, &neighbor, &survivor] {
        long.upsert_entity(e).await.unwrap();
    }
    let relation = Relationship {
        id: Uuid::new_v4(),
        source_id: shared.id,
        target_id: neighbor.id,
        relationship_type: "KNOWS".into(),
        description: None,
        confidence: 1.0,
        valid_from: None,
        valid_until: None,
        properties: Metadata::new(),
        created_at: Utc::now(),
    };
    long.upsert_relationship(&relation).await.unwrap();
    let retained_relation = Relationship {
        id: Uuid::new_v4(),
        source_id: neighbor.id,
        target_id: survivor.id,
        ..relation.clone()
    };
    long.upsert_relationship(&retained_relation).await.unwrap();
    let fact = Fact {
        id: Uuid::new_v4(),
        subject: "shared".into(),
        predicate: "knows".into(),
        object: "neighbor".into(),
        confidence: 1.0,
        embedding: None,
        valid_from: None,
        valid_until: None,
        created_at: Utc::now(),
        metadata: Metadata::new(),
    };
    long.create_fact(&fact).await.unwrap();
    for p in [&pa, &pb] {
        long.link_preference_to_entity(p.id, shared.id)
            .await
            .unwrap();
    }
    let session = format!("same-session-{}", Uuid::new_v4());
    let mut messages = Vec::new();
    let mut conversations = Vec::new();
    for (owner, content) in [(&owner_a, "A message"), (&owner_b, "B message")] {
        let conversation = short
            .ensure_conversation(&session, None, owner)
            .await
            .unwrap();
        let message = MessageRecord {
            id: Uuid::new_v4(),
            role: "user".into(),
            content: content.into(),
            embedding: Some(vec![1.0, 0.0, 0.0]),
            timestamp: Some(Utc::now()),
            metadata: Metadata::new(),
            embedding_identity: Some(EmbeddingIdentity::new("synthetic", 3).unwrap()),
            operation_key: None,
            operation_payload: None,
        };
        short
            .append_message(conversation, owner, &message)
            .await
            .unwrap();
        assert!(long
            .link_message_to_entity(message.id, shared.id, 1.0)
            .await
            .unwrap());
        messages.push(message);
        conversations.push(conversation);
    }
    assert_ne!(conversations[0], conversations[1]);
    assert_eq!(
        long.entity_mentions(shared.id)
            .await
            .unwrap()
            .unwrap()
            .len(),
        2
    );
    short.clear_session(&session, &owner_a).await.unwrap();
    assert!(short
        .get_conversation(&session, None, &owner_a, Some(10))
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        short
            .get_conversation(&session, None, &owner_b, Some(10))
            .await
            .unwrap()
            .unwrap()
            .messages[0]
            .id,
        messages[1].id
    );
    assert_eq!(
        long.entity_mentions(shared.id).await.unwrap().unwrap()[0].message_id,
        messages[1].id
    );
    assert_eq!(
        long.entity_mentions(shared.id)
            .await
            .unwrap()
            .unwrap()
            .len(),
        1
    );
    assert!(long.get_entity(shared.id).await.unwrap().is_some());
    assert_eq!(long.related_entities(shared.id, 1).await.unwrap().len(), 1);
    assert_eq!(
        count(
            &client,
            "MATCH (f:Fact {id: $id}) RETURN count(f) AS n",
            fact.id
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &client,
            "MATCH (:Preference)-[r:APPLIES_TO]->(:Entity {id: $id}) RETURN count(r) AS n",
            shared.id
        )
        .await,
        2
    );

    // Global DETACH DELETE: B's mention and all incident graph links disappear too.
    assert!(long.delete_entity(shared.id).await.unwrap());
    assert!(!long.delete_entity(shared.id).await.unwrap());
    assert!(long.get_entity(shared.id).await.unwrap().is_none());
    assert!(long.entity_mentions(shared.id).await.unwrap().is_none());
    assert_eq!(
        count(
            &client,
            "MATCH ()-[r:MENTIONS]->(:Entity {id: $id}) RETURN count(r) AS n",
            shared.id
        )
        .await,
        0
    );
    for e in [&neighbor, &survivor] {
        assert!(long.get_entity(e.id).await.unwrap().is_some());
    }
    assert_eq!(
        long.related_entities(neighbor.id, 1).await.unwrap()[0]
            .entity
            .id,
        survivor.id
    );
    assert_eq!(
        long.related_entities(neighbor.id, 1).await.unwrap().len(),
        1
    );
    assert_eq!(
        count(
            &client,
            "MATCH (f:Fact {id: $id}) RETURN count(f) AS n",
            fact.id
        )
        .await,
        1
    );
    let facts = long.facts_by_subject("shared", 100).await.unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].id, fact.id);
    assert_eq!(facts[0].predicate, fact.predicate);
    assert_eq!(facts[0].object, fact.object);
    for (user, expected) in [(&a, pa.id), (&b, pb.id)] {
        assert_eq!(
            long.preferences_for(user, true).await.unwrap()[0].id,
            expected
        );
        assert_eq!(
            count(
                &client,
                "MATCH (:Preference {id: $id})-[r:APPLIES_TO]->() RETURN count(r) AS n",
                expected
            )
            .await,
            0
        );
    }
    assert_eq!(
        short
            .get_conversation(&session, None, &owner_b, Some(10))
            .await
            .unwrap()
            .unwrap()
            .messages[0]
            .id,
        messages[1].id
    );
}
