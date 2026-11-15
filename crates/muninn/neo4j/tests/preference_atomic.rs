//! Pruebas con una instancia Neo4j sintética y desechable configurada explícitamente.
use chrono::Utc;
use muninn_core::model::{Metadata, Preference};
use muninn_core::ports::LongTermStore;
use muninn_neo4j::{Neo4jClient, Neo4jConfig, Neo4jLongTermStore, SchemaManager};
use uuid::Uuid;

async fn fixture() -> (Neo4jClient, Neo4jLongTermStore) {
    let client = Neo4jClient::connect(&Neo4jConfig {
        uri: std::env::var("MUNINN_FIXTURE_URI").unwrap(),
        password: std::env::var("MUNINN_FIXTURE_PASSWORD").unwrap(),
        ..Neo4jConfig::default()
    })
    .await
    .unwrap();
    SchemaManager::new(&client, 3).setup_all().await.unwrap();
    let store = Neo4jLongTermStore::new(client.clone());
    (client, store)
}

fn record(category: &str, value: &str) -> Preference {
    Preference {
        id: Uuid::new_v4(),
        category: category.into(),
        preference: value.into(),
        context: None,
        confidence: 1.0,
        embedding: Some(vec![1.0, 0.0, 0.0]),
        valid_from: None,
        valid_until: None,
        created_at: Utc::now(),
        metadata: Metadata::new(),
    }
}

#[tokio::test]
#[ignore = "requires explicitly configured fresh synthetic Neo4j"]
async fn scoped_search_and_dedup_survive_foreign_ann_saturation() {
    let (client, store) = fixture().await;
    let user = format!("owner-{}", Uuid::new_v4());
    let category = Uuid::new_v4().to_string();
    client.execute("UNWIND range(1,10001) AS i CREATE (p:Preference {id:randomUUID(), category:$category, preference:'foreign', embedding:[1.0,0.0,0.0]})", vec![("category", category.clone().into())]).await.unwrap();
    let own = record(&category, "tea");
    store.create_preference(&own).await.unwrap();
    store.link_user_to_preference(&user, own.id).await.unwrap();
    let hits = store
        .search_preferences(&[1.0, 0.0, 0.0], 3, 0.95, Some(&user))
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].item.id, own.id);
    assert_eq!(
        store
            .find_duplicate_preference(&[1.0, 0.0, 0.0], &category, 0.95, Some(&user))
            .await
            .unwrap()
            .unwrap()
            .id,
        own.id
    );
    // Un conjunto elegible excesivo no se convierte silenciosamente en vacío.
    assert!(store
        .find_duplicate_preference(&[1.0, 0.0, 0.0], &category, 0.95, None)
        .await
        .is_err());
}

#[tokio::test]
#[ignore = "requires explicitly configured fresh synthetic Neo4j"]
async fn missing_target_rolls_back_creation_links_and_supersession_including_equivalent_dedup() {
    let (client, store) = fixture().await;
    let user = format!("owner-{}", Uuid::new_v4());
    let category = Uuid::new_v4().to_string();
    let old = record(&category, "tea");
    store
        .write_preference(&old, Some(&user), &[], &[], 0.95)
        .await
        .unwrap();
    let target = Uuid::new_v4();
    client
        .execute(
            "CREATE (:Entity {id:$id, name:'target', type:'OBJECT'})",
            vec![("id", target.to_string().into())],
        )
        .await
        .unwrap();
    for value in ["coffee", " TEA "] {
        let new = record(&category, value);
        assert!(store
            .write_preference(&new, Some(&user), &[target], &[Uuid::new_v4()], 0.95)
            .await
            .is_err());
        let live = store.preferences_for(&user, true).await.unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].id, old.id);
        let rows = client.execute("MATCH (p:Preference) WHERE p.id IN [$old, $new] OPTIONAL MATCH (p)-[r:APPLIES_TO|ABOUT|SUPERSEDED_BY]->() RETURN count(DISTINCT p) AS nodes, count(r) AS edges", vec![("old", old.id.to_string().into()), ("new", new.id.to_string().into())]).await.unwrap();
        assert_eq!(rows[0].get::<i64>("nodes").unwrap(), 1);
        assert_eq!(rows[0].get::<i64>("edges").unwrap(), 0);
    }
    let equivalent = record(&category, " TEA ");
    let result = store
        .write_preference(&equivalent, Some(&user), &[target], &[target], 0.95)
        .await
        .unwrap();
    assert_eq!(result.id, old.id);
    assert_eq!(result.metadata["deduplicated"], true);
    let rows = client
        .execute(
            "MATCH (p:Preference {id:$id})-[r:APPLIES_TO|ABOUT]->() RETURN count(r) AS c",
            vec![("id", old.id.to_string().into())],
        )
        .await
        .unwrap();
    assert_eq!(rows[0].get::<i64>("c").unwrap(), 2);
}

#[tokio::test]
#[ignore = "requires explicitly configured fresh synthetic Neo4j"]
async fn competing_writers_repeat_lookup_under_database_lock() {
    let (client, store) = fixture().await;
    // Otro adaptador con su propio pool: un Mutex local no basta.
    let second_client = Neo4jClient::connect(&Neo4jConfig {
        uri: std::env::var("MUNINN_FIXTURE_URI").unwrap(),
        password: std::env::var("MUNINN_FIXTURE_PASSWORD").unwrap(),
        ..Neo4jConfig::default()
    })
    .await
    .unwrap();
    let second = Neo4jLongTermStore::new(second_client);
    let user = format!("owner-{}", Uuid::new_v4());
    let category = Uuid::new_v4().to_string();
    let a = record(&category, "tea");
    let b = record(&category, " TEA ");
    let (a, b) = tokio::join!(
        store.write_preference(&a, Some(&user), &[], &[], 0.95),
        second.write_preference(&b, Some(&user), &[], &[], 0.95)
    );
    assert_eq!(a.unwrap().id, b.unwrap().id);
    assert_eq!(store.preferences_for(&user, true).await.unwrap().len(), 1);
    let a = record(&category, "coffee");
    let b = record(&category, "water");
    let (a, b) = tokio::join!(
        store.write_preference(&a, Some(&user), &[], &[], 0.95),
        second.write_preference(&b, Some(&user), &[], &[], 0.95)
    );
    assert_ne!(a.unwrap().id, b.unwrap().id);
    assert_eq!(store.preferences_for(&user, true).await.unwrap().len(), 1);
    assert_eq!(store.preferences_for(&user, false).await.unwrap().len(), 3);
    // Confirmar una cadena sin bifurcaciones, no dos sucesores del mismo nodo.
    let rows = client.execute("MATCH (:User {identifier:$user})-[:HAS_PREFERENCE]->(p) OPTIONAL MATCH (p)-[r:SUPERSEDED_BY]->() WITH p, count(r) AS n RETURN max(n) AS maximum, sum(n) AS total", vec![("user", user.into())]).await.unwrap();
    assert_eq!(rows[0].get::<i64>("maximum").unwrap(), 1);
    assert_eq!(rows[0].get::<i64>("total").unwrap(), 2);
}
