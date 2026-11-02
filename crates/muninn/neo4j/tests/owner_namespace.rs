//! Opt-in integration test against a fresh, synthetic Neo4j fixture only.
use muninn_core::model::Metadata;
use muninn_core::ports::{MessageRecord, OwnerScope, SessionQuery, ShortTermStore};
use muninn_neo4j::{Neo4jClient, Neo4jConfig, Neo4jShortTermStore, SchemaManager};
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires explicitly configured fresh synthetic Neo4j"]
async fn owner_namespace_lifecycle() {
    let client = Neo4jClient::connect(&Neo4jConfig {
        uri: std::env::var("MUNINN_FIXTURE_URI").unwrap(),
        username: "neo4j".into(),
        password: std::env::var("MUNINN_FIXTURE_PASSWORD").unwrap(),
        ..Neo4jConfig::default()
    })
    .await
    .unwrap();
    SchemaManager::new(&client, 3).setup_all().await.unwrap();
    let store = Neo4jShortTermStore::new(client.clone());
    let a = OwnerScope::owned(&format!("a-{}", Uuid::new_v4())).unwrap();
    let b = OwnerScope::owned(&format!("b-{}", Uuid::new_v4())).unwrap();
    let u = OwnerScope::unowned();
    let session = Uuid::new_v4().to_string();
    let aid = store.ensure_conversation(&session, None, &a).await.unwrap();
    let bid = store.ensure_conversation(&session, None, &b).await.unwrap();
    let uid = store.ensure_conversation(&session, None, &u).await.unwrap();
    assert_ne!(aid, bid);
    assert_ne!(aid, uid);
    assert_ne!(bid, uid);
    assert!(store
        .ensure_conversation(&session, Some(aid), &b)
        .await
        .is_err());
    assert!(store
        .ensure_conversation("wrong-session", Some(aid), &a)
        .await
        .is_err());
    let record = MessageRecord {
        operation_key: None,
        operation_payload: None,
        id: Uuid::new_v4(),
        role: "user".into(),
        content: "owner-a-only".into(),
        embedding: Some(vec![1.0, 0.0, 0.0]),
        metadata: std::iter::once(("tag".into(), serde_json::json!("scoped"))).collect(),
        timestamp: None,
    };
    store.append_message(aid, &a, &record).await.unwrap();
    assert!(store.append_message(aid, &b, &record).await.is_err());
    let batch_record = MessageRecord {
        id: Uuid::new_v4(),
        ..record.clone()
    };
    assert!(store
        .append_messages(aid, &b, std::slice::from_ref(&batch_record))
        .await
        .is_err());
    let batch = store
        .append_messages(aid, &a, std::slice::from_ref(&batch_record))
        .await
        .unwrap();
    assert_eq!(batch.len(), 1);
    assert!(store
        .delete_message(batch_record.id, &a, true)
        .await
        .unwrap());
    assert!(store
        .get_conversation_by_id(aid, &b)
        .await
        .unwrap()
        .is_none());
    assert!(store
        .get_conversation("wrong-session", Some(aid), &a, None)
        .await
        .unwrap()
        .is_none());
    assert!(!store.delete_message(record.id, &b, true).await.unwrap());
    store.clear_session(&session, &b).await.unwrap();
    store.delete_conversation(aid, &b).await.unwrap();
    let own = store
        .get_conversation(&session, None, &a, Some(100))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(own.messages.len(), 1);
    let sessions = store
        .list_sessions(
            &SessionQuery {
                prefix: Some(session.clone()),
                limit: 10,
                ..SessionQuery::default()
            },
            &a,
        )
        .await
        .unwrap();
    assert_eq!(sessions.len(), 1);
    let distractor_session = Uuid::new_v4().to_string();
    let distractor_id = store
        .ensure_conversation(&distractor_session, None, &b)
        .await
        .unwrap();
    for _ in 0..20 {
        let mut distractor = record.clone();
        distractor.id = Uuid::new_v4();
        store
            .append_message(distractor_id, &b, &distractor)
            .await
            .unwrap();
    }
    let weaker = MessageRecord {
        id: Uuid::new_v4(),
        embedding: Some(vec![0.5, 0.5, 0.0]),
        ..record.clone()
    };
    store.append_message(aid, &a, &weaker).await.unwrap();
    let hits = store
        .search_messages(&[1.0, 0.0, 0.0], None, &a, 2, 0.0, &Metadata::new())
        .await
        .unwrap();
    assert_eq!(
        hits.len(),
        2,
        "foreign candidates must not starve owner results"
    );
    assert!(hits.iter().any(|hit| hit.message.id == weaker.id));
    let filters = std::iter::once(("tag".into(), serde_json::json!("scoped"))).collect();
    let hits = store
        .search_messages(&[1.0, 0.0, 0.0], None, &a, 2, 0.0, &filters)
        .await
        .unwrap();
    assert_eq!(
        hits.len(),
        2,
        "metadata search must not select global candidates"
    );
    assert!(store.delete_message(record.id, &a, true).await.unwrap());
    let concurrent_session = Uuid::new_v4().to_string();
    let (one, two) = tokio::join!(
        store.ensure_conversation(&concurrent_session, None, &a),
        store.ensure_conversation(&concurrent_session, None, &a),
    );
    assert_eq!(one.unwrap(), two.unwrap());
    // A legacy writer introduces an ambiguous sibling after scoped creation.
    client
        .execute(
            "CREATE (:Conversation {id: $id, session_id: $session})",
            vec![
                ("id", Uuid::new_v4().to_string().into()),
                ("session", session.clone().into()),
            ],
        )
        .await
        .unwrap();
    let ambiguous_record = MessageRecord {
        id: Uuid::new_v4(),
        ..record.clone()
    };
    assert!(store
        .append_message(aid, &a, &ambiguous_record)
        .await
        .is_err());
    assert!(store
        .append_messages(aid, &a, std::slice::from_ref(&ambiguous_record))
        .await
        .is_err());
    let legacy_session = Uuid::new_v4().to_string();
    let legacy_id = Uuid::new_v4();
    let legacy_message = Uuid::new_v4();
    client
        .execute(
            "CREATE (:Conversation {id: $id, session_id: $session})-[:HAS_MESSAGE]->(:Message {id: $message})",
            vec![
                ("id", legacy_id.to_string().into()),
                ("message", legacy_message.to_string().into()),
                ("session", legacy_session.clone().into()),
            ],
        )
        .await
        .unwrap();
    assert!(store
        .ensure_conversation(&legacy_session, None, &a)
        .await
        .is_err());
    assert!(store
        .get_conversation(&legacy_session, None, &a, None)
        .await
        .is_err());
    assert!(store.clear_session(&legacy_session, &a).await.is_err());
    assert!(store.delete_conversation(legacy_id, &a).await.is_err());
    assert!(store
        .delete_message(legacy_message, &a, true)
        .await
        .is_err());
    assert!(store
        .search_messages(
            &[1.0, 0.0, 0.0],
            Some(&legacy_session),
            &a,
            2,
            0.0,
            &Metadata::new()
        )
        .await
        .is_err());
}

#[tokio::test]
#[ignore = "requires explicitly configured fresh synthetic Neo4j"]
async fn guarded_mutation_rolls_back_on_conflict() {
    let client = Neo4jClient::connect(&Neo4jConfig {
        uri: std::env::var("MUNINN_FIXTURE_URI").unwrap(),
        username: "neo4j".into(),
        password: std::env::var("MUNINN_FIXTURE_PASSWORD").unwrap(),
        ..Neo4jConfig::default()
    })
    .await
    .unwrap();
    let id = Uuid::new_v4().to_string();
    let result = client
        .execute_guarded(
            "MATCH (c:Conversation {id: $id}) RETURN c LIMIT 1",
            vec![("id", id.clone().into())],
            "CREATE (c:Conversation {id: $id}) RETURN c",
            vec![("id", id.clone().into())],
        )
        .await;
    assert!(result.is_err());
    assert!(client
        .execute(
            "MATCH (c:Conversation {id: $id}) RETURN c",
            vec![("id", id.into())]
        )
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
#[ignore = "requires explicitly configured fresh synthetic Neo4j"]
async fn operation_key_replays_and_rejects_conflicts() {
    let client = Neo4jClient::connect(&Neo4jConfig {
        uri: std::env::var("MUNINN_FIXTURE_URI").unwrap(),
        username: "neo4j".into(),
        password: std::env::var("MUNINN_FIXTURE_PASSWORD").unwrap(),
        ..Neo4jConfig::default()
    })
    .await
    .unwrap();
    SchemaManager::new(&client, 3).setup_all().await.unwrap();
    let store = Neo4jShortTermStore::new(client.clone());
    let owner = OwnerScope::owned(&Uuid::new_v4().to_string()).unwrap();
    let session = Uuid::new_v4().to_string();
    let conversation = store
        .ensure_conversation(&session, None, &owner)
        .await
        .unwrap();
    let incomplete_keyed_record = MessageRecord {
        id: Uuid::new_v4(),
        role: "user".into(),
        content: "missing payload".into(),
        embedding: None,
        metadata: Metadata::new(),
        timestamp: None,
        operation_key: Some("incomplete-keyed-record".into()),
        operation_payload: None,
    };
    assert!(store
        .append_message(conversation, &owner, &incomplete_keyed_record)
        .await
        .is_err());
    assert!(store
        .append_messages(
            conversation,
            &owner,
            std::slice::from_ref(&incomplete_keyed_record)
        )
        .await
        .is_err());
    let record = MessageRecord {
        id: Uuid::new_v4(),
        role: "user".into(),
        content: "same".into(),
        embedding: None,
        metadata: Metadata::new(),
        timestamp: None,
        operation_key: Some("operation-1".into()),
        operation_payload: Some("payload-1".into()),
    };
    assert!(store
        .append_messages(conversation, &owner, std::slice::from_ref(&record))
        .await
        .is_err());
    let first = store
        .append_message(conversation, &owner, &record)
        .await
        .unwrap();
    let found = store
        .find_by_operation_key(conversation, &owner, "operation-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.message.id, first.id);
    assert_eq!(found.operation_payload.as_deref(), Some("payload-1"));
    let other_owner = OwnerScope::owned(&Uuid::new_v4().to_string()).unwrap();
    assert!(store
        .find_by_operation_key(conversation, &other_owner, "operation-1")
        .await
        .unwrap()
        .is_none());
    let retry = MessageRecord {
        id: Uuid::new_v4(),
        ..record.clone()
    };
    let second = store
        .append_message(conversation, &owner, &retry)
        .await
        .unwrap();
    assert_eq!(first.id, second.id);
    let changed = MessageRecord {
        id: Uuid::new_v4(),
        content: "different".into(),
        operation_payload: Some("payload-2".into()),
        ..record.clone()
    };
    assert!(store
        .append_message(conversation, &owner, &changed)
        .await
        .is_err());
    let (one, two) = tokio::join!(
        store.append_message(conversation, &owner, &retry),
        store.append_message(conversation, &owner, &retry)
    );
    assert_eq!(one.unwrap().id, two.unwrap().id);
    let fresh_one = MessageRecord {
        id: Uuid::new_v4(),
        operation_key: Some("fresh-concurrent".into()),
        ..record.clone()
    };
    let fresh_two = MessageRecord {
        id: Uuid::new_v4(),
        ..fresh_one.clone()
    };
    let (one, two) = tokio::join!(
        store.append_message(conversation, &owner, &fresh_one),
        store.append_message(conversation, &owner, &fresh_two)
    );
    assert_eq!(one.unwrap().id, two.unwrap().id);
    assert_keyed_chain(&client, conversation, "fresh-concurrent", 1, 2).await;

    let conflict_one = MessageRecord {
        id: Uuid::new_v4(),
        operation_key: Some("concurrent-conflict".into()),
        operation_payload: Some("payload-a".into()),
        content: "payload a".into(),
        ..record.clone()
    };
    let conflict_two = MessageRecord {
        id: Uuid::new_v4(),
        operation_payload: Some("payload-b".into()),
        content: "payload b".into(),
        ..conflict_one.clone()
    };
    let (left, right) = tokio::join!(
        store.append_message(conversation, &owner, &conflict_one),
        store.append_message(conversation, &owner, &conflict_two)
    );
    assert_ne!(left.is_ok(), right.is_ok());
    assert_keyed_chain(&client, conversation, "concurrent-conflict", 1, 3).await;
    let other = store
        .ensure_conversation(&Uuid::new_v4().to_string(), None, &owner)
        .await
        .unwrap();
    let other_record = MessageRecord {
        id: Uuid::new_v4(),
        ..retry.clone()
    };
    assert_ne!(
        store
            .append_message(other, &owner, &other_record)
            .await
            .unwrap()
            .id,
        first.id
    );
    let ordinary = MessageRecord {
        id: Uuid::new_v4(),
        operation_key: None,
        operation_payload: None,
        ..record.clone()
    };
    let ordinary_two = MessageRecord {
        id: Uuid::new_v4(),
        ..ordinary.clone()
    };
    assert_ne!(
        store
            .append_message(conversation, &owner, &ordinary)
            .await
            .unwrap()
            .id,
        store
            .append_message(conversation, &owner, &ordinary_two)
            .await
            .unwrap()
            .id
    );
    assert!(store.delete_message(first.id, &owner, true).await.unwrap());
    let recreated = store
        .append_message(conversation, &owner, &retry)
        .await
        .unwrap();
    assert_eq!(recreated.id, retry.id);
}

async fn assert_keyed_chain(
    client: &Neo4jClient,
    conversation_id: Uuid,
    operation_key: &str,
    expected_keyed_messages: i64,
    expected_total_messages: i64,
) {
    let keyed_rows = client
        .execute(
            "MATCH (c:Conversation {id: $id})-[:HAS_MESSAGE]->(m:Message {operation_key: $key})
             RETURN {count: count(m)} AS stats",
            vec![
                ("id", conversation_id.to_string().into()),
                ("key", operation_key.into()),
            ],
        )
        .await
        .unwrap();
    let keyed_stats_value =
        muninn_neo4j::bolt::column(keyed_rows.first().expect("key count row"), "stats").unwrap();
    let keyed_stats = muninn_neo4j::bolt::Props::of(&keyed_stats_value).unwrap();
    assert_eq!(keyed_stats.i64("count"), Some(expected_keyed_messages));

    let chain_rows = client
        .execute(
            "MATCH (c:Conversation {id: $id})
             OPTIONAL MATCH (c)-[:HAS_MESSAGE]->(m:Message)
             WITH c, count(m) AS total, collect(m) AS messages
             OPTIONAL MATCH (c)-[:FIRST_MESSAGE]->(first:Message)
             WITH total, messages, count(first) AS roots
             UNWIND CASE WHEN size(messages) = 0 THEN [null] ELSE messages END AS m
             OPTIONAL MATCH (m)-[:NEXT_MESSAGE]->(next)
             WITH total, roots, m, count(next) AS degree
             WITH total, roots,
                  sum(CASE WHEN degree > 1 THEN 1 ELSE 0 END) AS forks,
                  sum(degree) AS links
             RETURN {total: total, roots: roots, forks: forks, links: links} AS stats",
            vec![("id", conversation_id.to_string().into())],
        )
        .await
        .unwrap();
    let chain_stats_value =
        muninn_neo4j::bolt::column(chain_rows.first().expect("chain statistics row"), "stats")
            .unwrap();
    let chain_stats = muninn_neo4j::bolt::Props::of(&chain_stats_value).unwrap();
    assert_eq!(chain_stats.i64("total"), Some(expected_total_messages));
    assert_eq!(chain_stats.i64("roots"), Some(1));
    assert_eq!(chain_stats.i64("forks"), Some(0));
    assert_eq!(chain_stats.i64("links"), Some(expected_total_messages - 1));

    let reachability_rows = client
        .execute(
            "MATCH (c:Conversation {id: $id})-[:FIRST_MESSAGE]->(root:Message)
             MATCH (root)-[:NEXT_MESSAGE*0..]->(reachable:Message)
             WHERE EXISTS { MATCH (c)-[:HAS_MESSAGE]->(reachable) }
             RETURN {count: count(DISTINCT reachable)} AS stats",
            vec![("id", conversation_id.to_string().into())],
        )
        .await
        .unwrap();
    let reachability_value = muninn_neo4j::bolt::column(
        reachability_rows
            .first()
            .expect("reachability statistics row"),
        "stats",
    )
    .unwrap();
    let reachability_stats = muninn_neo4j::bolt::Props::of(&reachability_value).unwrap();
    assert_eq!(
        reachability_stats.i64("count"),
        Some(expected_total_messages)
    );
}
