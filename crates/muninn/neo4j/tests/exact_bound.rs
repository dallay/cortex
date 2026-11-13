//! Run only on a new disposable Neo4j instance.
use muninn_core::model::Metadata;
use muninn_core::ports::{EmbeddingIdentity, OwnerScope, ShortTermStore};
use muninn_core::MemoryError;
use muninn_neo4j::{Neo4jClient, Neo4jConfig, Neo4jShortTermStore, SchemaManager};

#[tokio::test]
#[ignore = "requires explicitly configured fresh synthetic Neo4j"]
async fn exact_bound_filters_before_probe_and_rejects_10001_without_scoring() {
    let client = Neo4jClient::connect(&Neo4jConfig {
        uri: std::env::var("MUNINN_FIXTURE_URI").unwrap(),
        password: std::env::var("MUNINN_FIXTURE_PASSWORD").unwrap(),
        ..Neo4jConfig::default()
    })
    .await
    .unwrap();
    SchemaManager::new(&client, 3).setup_all().await.unwrap();
    // Foreign identity/owner/session/metadata candidates must not consume probe budget.
    client.execute("UNWIND range(0, 10000) AS i CREATE (c:Conversation {id:toString(i), owner_scope:'owned:foreign', session_id:'foreign-'+toString(i)}) CREATE (m:Message {id:toString(i), embedding:[0.0,1.0,0.0], embedding_dimensions:3, embedding_identity:'other', metadata:'{}'}) CREATE (c)-[:HAS_MESSAGE]->(m)", vec![]).await.unwrap();
    client.execute("CREATE (c:Conversation {id:'eligible', owner_scope:'owned:test', session_id:'target'}) WITH c UNWIND range(0,9999) AS i CREATE (m:Message {id:randomUUID(), role:'user', content:'fixture', embedding:[1.0,0.0,0.0], embedding_dimensions:3, embedding_identity:'bound:test', metadata:'{\"tag\": \"allowed\"}'}) CREATE (c)-[:HAS_MESSAGE]->(m)", vec![]).await.unwrap();
    client
        .execute("CALL db.awaitIndexes(120)", vec![])
        .await
        .unwrap();
    let store = Neo4jShortTermStore::new(client.clone());
    let owner = OwnerScope::owned("test").unwrap();
    let identity = EmbeddingIdentity::new("bound:test", 3).unwrap();
    let filters: Metadata = std::iter::once(("tag".into(), serde_json::json!("allowed"))).collect();
    // Force fallback without relying on HNSW ordering: threshold excludes every ANN hit.
    // EXACT still counts eligible candidates before scoring/threshold, even for empty results.
    for session in [None, Some("target")] {
        for metadata in [Metadata::new(), filters.clone()] {
            let result = store
                .search_messages_with_identity(
                    &[1.0, 0.0, 0.0],
                    &identity,
                    session,
                    &owner,
                    10,
                    1.1,
                    &metadata,
                )
                .await
                .unwrap();
            assert!(result.is_empty());
        }
    }
    // Direct current query verifies scoring at the boundary without asking HNSW
    // to materialize a huge top-k (Neo4j fixture memory is deliberately limited).
    let complete = client
        .execute(
            muninn_neo4j::queries::short_term::SEARCH_MESSAGES_EXACT_IN_SESSION,
            vec![
                ("embedding", vec![1.0_f64, 0.0, 0.0].into()),
                ("embedding_identity", "bound:test".into()),
                ("embedding_dimensions", 3_i64.into()),
                ("owner_scope", owner.storage_key().into()),
                ("session_id", "target".into()),
                ("limit", 10_i64.into()),
                ("threshold", 0.0_f64.into()),
                ("candidate_probe_limit", 10_001_i64.into()),
                ("max_candidates", 10_000_i64.into()),
            ],
        )
        .await
        .unwrap();
    assert_eq!(complete.len(), 10);
    for row in complete {
        assert!(!row.get::<bool>("over_limit").unwrap());
        assert!((row.get::<f64>("score").unwrap() - 1.0).abs() < 1e-6);
    }
    // Each filter independently removes a population larger than the probe.
    client.execute("CREATE (c:Conversation {id:'other-session', owner_scope:'owned:test', session_id:'elsewhere'}) WITH c UNWIND range(0,10000) AS i CREATE (m:Message {id:randomUUID(), embedding:[1.0,0.0,0.0], embedding_dimensions:3, embedding_identity:'bound:test', metadata:'{\"tag\": \"denied\"}'}) CREATE (c)-[:HAS_MESSAGE]->(m)",vec![]).await.unwrap();
    for (session, metadata) in [(Some("target"), Metadata::new()), (None, filters.clone())] {
        assert!(store
            .search_messages_with_identity(
                &[1.0, 0.0, 0.0],
                &identity,
                session,
                &owner,
                10,
                1.1,
                &metadata
            )
            .await
            .unwrap()
            .is_empty());
    }
    client
        .execute(
            "MATCH (c:Conversation {id:'other-session'})-[:HAS_MESSAGE]->(m) DETACH DELETE m,c",
            vec![],
        )
        .await
        .unwrap();
    client.execute("MATCH (c:Conversation {id:'eligible'}) CREATE (m:Message {id:randomUUID(), role:'user', content:'overflow', embedding:[0.0,0.0,0.0], embedding_dimensions:3, embedding_identity:'bound:test', metadata:'{\"tag\": \"allowed\"}'}) CREATE (c)-[:HAS_MESSAGE]->(m)",vec![]).await.unwrap();
    for session in [None, Some("target")] {
        for metadata in [Metadata::new(), filters.clone()] {
            let result = store
                .search_messages_with_identity(
                    &[1.0, 0.0, 0.0],
                    &identity,
                    session,
                    &owner,
                    10,
                    1.1,
                    &metadata,
                )
                .await;
            assert!(
                matches!(result, Err(MemoryError::Storage(ref message)) if message.contains("EXACT candidate set exceeds 10000")),
                "expected explicit capacity error, got {result:?}"
            );
        }
    }
    println!("PASS 10000 accepted, 10001 rejected across all four scoped variants");
}
