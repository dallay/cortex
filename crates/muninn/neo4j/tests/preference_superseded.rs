//! Run only against an explicitly configured, fresh synthetic Neo4j fixture.
//!
//! Proves that a superseded preference stays in the graph as history while
//! every live read path (embedding search, category search and the duplicate
//! lookup used by writes) hides it.
use async_trait::async_trait;
use muninn_application::{AddPreferenceOptions, LongTermMemory, SearchPreferencesOptions};
use muninn_core::model::{Metadata, Preference};
use muninn_core::ports::{Embedder, LongTermStore};
use muninn_core::MemoryResult;
use muninn_neo4j::{Neo4jClient, Neo4jConfig, Neo4jLongTermStore, SchemaManager};
use uuid::Uuid;

fn embedding(seed: f64) -> Vec<f64> {
    vec![seed, 1.0 - seed, 0.5]
}

/// Stable hash of `text` in `[0, 1)`.
fn hash_unit(text: &str) -> f64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    (hash % 1_000) as f64 / 1_000.0
}

/// Embedder with a dominant category component and a tiny value component.
///
/// Two values of one category stay above the duplicate threshold, while
/// another category stays far below it. That keeps the fixture deterministic:
/// the store's duplicate lookup only considers the globally nearest
/// candidates, so the preference of this test is always among them.
struct CategoryDominantEmbedder;

#[async_trait]
impl Embedder for CategoryDominantEmbedder {
    fn dimensions(&self) -> usize {
        3
    }
    async fn embed(&self, text: &str) -> MemoryResult<Vec<f64>> {
        let (category, value) = text.split_once(": ").unwrap_or((text, ""));
        Ok(vec![
            1.0,
            hash_unit(category).mul_add(1.998, -0.999),
            hash_unit(value).mul_add(2.0, -1.0) * 0.000_001,
        ])
    }
}

async fn fixture() -> (Neo4jClient, Neo4jLongTermStore) {
    let client = Neo4jClient::connect(&Neo4jConfig {
        uri: std::env::var("MUNINN_FIXTURE_URI").unwrap(),
        username: "neo4j".into(),
        password: std::env::var("MUNINN_FIXTURE_PASSWORD").unwrap(),
        ..Neo4jConfig::default()
    })
    .await
    .unwrap();
    // The vector indexes must exist for the embedding read paths.
    SchemaManager::new(&client, 3).setup_all().await.unwrap();
    let store = Neo4jLongTermStore::new(client.clone());
    (client, store)
}

async fn superseded_edges(client: &Neo4jClient, id: Uuid) -> i64 {
    let rows = client
        .execute(
            "MATCH (p:Preference {id: $id})-[:SUPERSEDED_BY]->(:Preference) RETURN count(p) AS c",
            vec![("id", id.to_string().into())],
        )
        .await
        .unwrap();
    match rows[0].get("c").ok() {
        Some(neo4rs::BoltType::Integer(value)) => value.value,
        other => panic!("unexpected count column: {other:?}"),
    }
}

#[tokio::test]
#[ignore = "requires explicitly configured fresh synthetic Neo4j"]
async fn superseded_preferences_are_hidden_from_every_live_read_path() {
    let (client, store) = fixture().await;
    let user = format!("superseded-{}", Uuid::new_v4());
    let category = format!("category-{}", Uuid::new_v4());
    let old = Preference {
        id: Uuid::new_v4(),
        category: category.clone(),
        preference: "The default indentation of the project is 2 spaces.".into(),
        context: None,
        confidence: 1.0,
        embedding: Some(embedding(0.1)),
        valid_from: None,
        valid_until: None,
        created_at: chrono::Utc::now(),
        metadata: Metadata::new(),
    };
    let new = Preference {
        id: Uuid::new_v4(),
        category: category.clone(),
        preference: "The default indentation of the project is 4 spaces.".into(),
        context: None,
        confidence: 1.0,
        embedding: Some(embedding(0.2)),
        valid_from: None,
        valid_until: None,
        created_at: chrono::Utc::now(),
        metadata: Metadata::new(),
    };
    store.create_preference(&old).await.unwrap();
    store.create_preference(&new).await.unwrap();
    store.link_user_to_preference(&user, old.id).await.unwrap();
    store.link_user_to_preference(&user, new.id).await.unwrap();

    // Before the supersede both records are live.
    let live: Vec<Uuid> = store
        .preferences_for(&user, true)
        .await
        .unwrap()
        .iter()
        .map(|p| p.id)
        .collect();
    assert!(live.contains(&old.id) && live.contains(&new.id));

    store.supersede_preference(old.id, new.id).await.unwrap();

    // The superseded record is gone from every live read path.
    let active: Vec<Uuid> = store
        .preferences_for(&user, true)
        .await
        .unwrap()
        .iter()
        .map(|p| p.id)
        .collect();
    assert!(!active.contains(&old.id), "{active:?}");
    assert!(active.contains(&new.id), "{active:?}");

    let hits = store
        .search_preferences(&embedding(0.1), 100, 0.0, Some(user.as_str()))
        .await
        .unwrap();
    assert!(
        hits.iter().all(|hit| hit.item.id != old.id),
        "embedding search returned the superseded preference"
    );
    assert!(hits.iter().any(|hit| hit.item.id == new.id));

    let by_category = store
        .preferences_by_category(&category, 100, Some(user.as_str()))
        .await
        .unwrap();
    assert!(
        by_category.iter().all(|p| p.id != old.id),
        "category search returned the superseded preference"
    );
    assert!(by_category.iter().any(|p| p.id == new.id));

    // The duplicate lookup must not return the superseded record either, or a
    // repeated write would be folded into a hidden record and lost.
    let duplicate = store
        .find_duplicate_preference(&embedding(0.1), &category, 0.0, Some(user.as_str()))
        .await
        .unwrap();
    assert!(
        duplicate.as_ref().is_none_or(|p| p.id != old.id),
        "duplicate lookup returned the superseded preference"
    );

    // History is preserved: the record and its edge still exist in the graph.
    assert_eq!(superseded_edges(&client, old.id).await, 1);
}

#[tokio::test]
#[ignore = "requires explicitly configured fresh synthetic Neo4j"]
async fn a_changed_preference_value_is_stored_and_supersedes_the_old_record() {
    let (client, store) = fixture().await;
    let memory = LongTermMemory::new(
        std::sync::Arc::new(store),
        Some(std::sync::Arc::new(CategoryDominantEmbedder)),
        false,
    );
    let user = format!("update-{}", Uuid::new_v4());
    let category = format!("category-{}", Uuid::new_v4());
    let options = || AddPreferenceOptions {
        user_identifier: Some(user.clone()),
        ..AddPreferenceOptions::default()
    };

    let first = memory
        .add_preference(
            &category,
            "The default indentation of the project is 2 spaces.",
            options(),
        )
        .await
        .unwrap();
    let second = memory
        .add_preference(
            &category,
            "The default indentation of the project is 4 spaces.",
            options(),
        )
        .await
        .unwrap();

    // The new value is stored, not deduplicated away.
    assert_ne!(second.id, first.id);
    assert_eq!(
        second.preference,
        "The default indentation of the project is 4 spaces."
    );

    // The old record is superseded and hidden from every live read path.
    assert_eq!(superseded_edges(&client, first.id).await, 1);

    let hits = memory
        .search_preferences(
            &format!("{category}: The default indentation of the project is 4 spaces."),
            &SearchPreferencesOptions {
                user_identifier: Some(user.clone()),
                category: Some(category.clone()),
                limit: 100,
                ..SearchPreferencesOptions::default()
            },
        )
        .await
        .unwrap();
    assert!(
        hits.iter().all(|hit| hit.id != first.id),
        "search returned the superseded preference"
    );
    assert!(hits.iter().any(|hit| hit.id == second.id));
}
