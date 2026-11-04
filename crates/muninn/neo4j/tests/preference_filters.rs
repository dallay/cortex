//! Run only against an explicitly configured, fresh synthetic Neo4j fixture.
use chrono::{DateTime, Utc};
use muninn_core::model::{Entity, Metadata, Preference};
use muninn_core::ports::LongTermStore;
use muninn_neo4j::{Neo4jClient, Neo4jConfig, Neo4jLongTermStore};
use uuid::Uuid;

fn time(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

async fn ids(
    store: &Neo4jLongTermStore,
    user: &str,
    active: bool,
    target: Option<Uuid>,
    instant: Option<DateTime<Utc>>,
) -> Vec<Uuid> {
    store
        .preferences_for_filtered(user, active, target, instant)
        .await
        .unwrap()
        .iter()
        .map(|p| p.id)
        .collect()
}

#[tokio::test]
#[ignore = "requires explicitly configured fresh synthetic Neo4j"]
async fn preference_filters_and_boundaries_distinct_history_and_unknowns() {
    let client = Neo4jClient::connect(&Neo4jConfig {
        uri: std::env::var("MUNINN_FIXTURE_URI").unwrap(),
        username: "neo4j".into(),
        password: std::env::var("MUNINN_FIXTURE_PASSWORD").unwrap(),
        ..Neo4jConfig::default()
    })
    .await
    .unwrap();
    let store = Neo4jLongTermStore::new(client.clone());
    let user = format!("filters-{}", Uuid::new_v4());
    let other = format!("filters-{}", Uuid::new_v4());
    let target = Entity {
        id: Uuid::new_v4(),
        name: format!("target-{}", Uuid::new_v4()),
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
    };
    store.upsert_entity(&target).await.unwrap();
    let unlinked = Entity {
        id: Uuid::new_v4(),
        name: format!("unlinked-{}", Uuid::new_v4()),
        ..target.clone()
    };
    store.upsert_entity(&unlinked).await.unwrap();
    // Explicit fixture timestamps make ordering and all four null-bound combinations deterministic.
    let mut preferences = Vec::new();
    for (index, (from, until, owner, linked)) in [
        (None, None, user.as_str(), true),
        (Some("2026-01-01T00:00:00Z"), None, user.as_str(), true),
        (None, Some("2026-01-02T00:00:00Z"), user.as_str(), true),
        (
            Some("2026-01-01T00:00:00Z"),
            Some("2026-01-02T00:00:00Z"),
            user.as_str(),
            true,
        ),
        (None, None, other.as_str(), true),
        (None, None, user.as_str(), false),
    ]
    .into_iter()
    .enumerate()
    {
        let p = Preference {
            id: Uuid::new_v4(),
            category: user.clone(),
            preference: format!("preference {index}"),
            context: None,
            confidence: 1.0,
            embedding: None,
            valid_from: None,
            valid_until: None,
            created_at: Utc::now(),
            metadata: Metadata::new(),
        };
        store.create_preference(&p).await.unwrap();
        store.link_user_to_preference(owner, p.id).await.unwrap();
        if linked {
            store
                .link_preference_to_entity(p.id, target.id)
                .await
                .unwrap();
            store
                .link_preference_to_entity(p.id, target.id)
                .await
                .unwrap();
        }
        client.execute(
            "MATCH (p:Preference {id: $id}) SET p.valid_from = datetime($from), p.valid_until = datetime($until), p.created_at = datetime($created)",
            vec![
                ("id", p.id.to_string().into()),
                ("from", from.map_or(neo4rs::BoltType::Null(neo4rs::BoltNull), Into::into)),
                ("until", until.map_or(neo4rs::BoltType::Null(neo4rs::BoltNull), Into::into)),
                ("created", format!("2025-01-01T00:00:0{index}Z").into()),
            ],
        ).await.unwrap();
        preferences.push(p.id);
    }
    // Duplicate graph edges must not multiply the public result, even without a target filter.
    client.execute(
        "MATCH (u:User {identifier: $user})-[:HAS_PREFERENCE]->(p:Preference {id: $id}), (e:Entity {id: $target}) CREATE (u)-[:HAS_PREFERENCE]->(p) CREATE (p)-[:APPLIES_TO]->(e)",
        vec![("user", user.as_str().into()), ("id", preferences[0].to_string().into()), ("target", target.id.to_string().into())],
    ).await.unwrap();
    let [open, lower, upper, bounded, _foreign, no_target] = preferences.as_slice() else {
        unreachable!()
    };
    let before = time("2025-12-31T23:59:59Z");
    let start = time("2026-01-01T00:00:00Z");
    let end = time("2026-01-02T00:00:00Z");
    for (instant, expected) in [
        (before, vec![*upper, *open]),
        (start, vec![*bounded, *upper, *lower, *open]),
        (end, vec![*lower, *open]),
    ] {
        assert_eq!(
            ids(&store, &user, true, Some(target.id), Some(instant)).await,
            expected
        );
    }
    assert_eq!(
        ids(&store, &user, true, None, Some(start)).await,
        vec![*no_target, *bounded, *upper, *lower, *open]
    );
    assert_eq!(
        ids(&store, &user, true, Some(target.id), None).await,
        vec![*bounded, *upper, *lower, *open]
    );
    assert_eq!(
        store
            .preferences_for(&user, true)
            .await
            .unwrap()
            .iter()
            .map(|p| p.id)
            .collect::<Vec<_>>(),
        vec![*no_target, *bounded, *upper, *lower, *open]
    );
    for missing in [unlinked.id, Uuid::new_v4()] {
        assert!(ids(&store, &user, false, Some(missing), None)
            .await
            .is_empty());
    }
    assert!(
        ids(&store, "unknown-user", false, Some(target.id), Some(start))
            .await
            .is_empty()
    );
    store.supersede_preference(*bounded, *lower).await.unwrap();
    // Superseding does not move an already-set valid_until; history remains half-open.
    assert_eq!(
        ids(&store, &user, true, Some(target.id), Some(start)).await,
        vec![*upper, *lower, *open]
    );
    assert_eq!(
        ids(&store, &user, false, Some(target.id), Some(start)).await,
        vec![*bounded, *upper, *lower, *open]
    );
    assert_eq!(
        ids(&store, &user, false, Some(target.id), Some(end)).await,
        vec![*lower, *open]
    );
}
