//! Opt-in tests. Use only a newly created disposable Neo4j fixture.
use muninn_core::model::Metadata;
use muninn_core::ports::{EmbeddingIdentity, MessageRecord, OwnerScope, ShortTermStore};
use muninn_core::MemoryError;
use muninn_neo4j::{Neo4jClient, Neo4jConfig, Neo4jShortTermStore, SchemaManager};
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires explicitly configured fresh synthetic Neo4j"]
async fn ann_underfill_restores_all_scoped_variants_and_propagates_errors() {
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
    let foreign = OwnerScope::owned(&Uuid::new_v4().to_string()).unwrap();
    let session = Uuid::new_v4().to_string();
    let conversation = store
        .ensure_conversation(&session, None, &owner)
        .await
        .unwrap();
    let other = store
        .ensure_conversation(&session, None, &foreign)
        .await
        .unwrap();
    let identity = EmbeddingIdentity::new("ann-fallback:test", 3).unwrap();
    let filters: Metadata = std::iter::once(("tag".into(), serde_json::json!("allowed"))).collect();
    let record = MessageRecord {
        operation_key: None,
        operation_payload: None,
        id: Uuid::new_v4(),
        role: "user".into(),
        content: "scoped".into(),
        embedding: Some(vec![0.5, 0.5, 0.0]),
        embedding_identity: Some(identity.clone()),
        metadata: filters.clone(),
        timestamp: None,
    };
    store
        .append_message(conversation, &owner, &record)
        .await
        .unwrap();
    let second = MessageRecord {
        id: Uuid::new_v4(),
        embedding: Some(vec![0.4, 0.6, 0.0]),
        ..record.clone()
    };
    store
        .append_message(conversation, &owner, &second)
        .await
        .unwrap();
    for _ in 0..20 {
        let distractor = MessageRecord {
            id: Uuid::new_v4(),
            embedding: Some(vec![1.0, 0.0, 0.0]),
            ..record.clone()
        };
        store
            .append_message(other, &foreign, &distractor)
            .await
            .unwrap();
    }
    client
        .execute("CALL db.awaitIndexes(60)", vec![])
        .await
        .unwrap();
    for selected_session in [None, Some(session.as_str())] {
        for selected_filters in [&Metadata::new(), &filters] {
            let hits = store
                .search_messages_with_identity(
                    &[1.0, 0.0, 0.0],
                    &identity,
                    selected_session,
                    &owner,
                    2,
                    0.0,
                    selected_filters,
                )
                .await
                .unwrap();
            assert_eq!(hits.len(), 2, "underfill must recover scoped results");
            assert_eq!(hits[0].message.id, record.id);
            assert_eq!(hits[1].message.id, second.id);
            let hits = store
                .search_messages_with_identity(
                    &[1.0, 0.0, 0.0],
                    &identity,
                    selected_session,
                    &owner,
                    2,
                    0.9,
                    selected_filters,
                )
                .await
                .unwrap();
            assert!(hits.is_empty(), "fallback must preserve threshold");
        }
    }
    let missing_tag: Metadata =
        std::iter::once(("tag".into(), serde_json::json!("denied"))).collect();
    for selected_session in [None, Some(session.as_str())] {
        let hits = store
            .search_messages_with_identity(
                &[1.0, 0.0, 0.0],
                &identity,
                selected_session,
                &owner,
                2,
                0.0,
                &missing_tag,
            )
            .await
            .unwrap();
        assert!(hits.is_empty(), "fallback must preserve metadata");
        let other_identity = EmbeddingIdentity::new("ann-fallback:other", 3).unwrap();
        let hits = store
            .search_messages_with_identity(
                &[1.0, 0.0, 0.0],
                &other_identity,
                selected_session,
                &owner,
                2,
                0.0,
                &Metadata::new(),
            )
            .await
            .unwrap();
        assert!(hits.is_empty(), "fallback must preserve embedding identity");
    }
    for selected_filters in [&Metadata::new(), &filters] {
        let hits = store
            .search_messages_with_identity(
                &[1.0, 0.0, 0.0],
                &identity,
                Some("absent-session"),
                &owner,
                2,
                0.0,
                selected_filters,
            )
            .await
            .unwrap();
        assert!(hits.is_empty(), "fallback must preserve session");
    }
    // A vector valid for its identity still fails against a differently sized index.
    let wrong_dimensions = EmbeddingIdentity::new("ann-fallback:two", 2).unwrap();
    assert!(matches!(
        store
            .search_messages_with_identity(
                &[1.0, 0.0],
                &wrong_dimensions,
                None,
                &owner,
                2,
                0.0,
                &Metadata::new()
            )
            .await,
        Err(MemoryError::Storage(_))
    ));
    // Dropping an index is permitted only inside this disposable fixture.
    client
        .execute("DROP INDEX message_embedding_idx", vec![])
        .await
        .unwrap();
    assert!(matches!(
        store
            .search_messages_with_identity(
                &[1.0, 0.0, 0.0],
                &identity,
                None,
                &owner,
                2,
                0.0,
                &Metadata::new()
            )
            .await,
        Err(MemoryError::Storage(_))
    ));
}
