//! BOLT-08-02: summary entities stay within the conversation owner namespace.
use std::sync::Arc;

use muninn_application::{AddMessageOptions, MemoryClient, MemoryOptions, MemoryPorts};
use muninn_core::extraction::ExtractionMode;
use muninn_core::model::{EntityRef, MessageRole};
use muninn_core::ports::{OwnerScope, ShortTermStore};
use muninn_neo4j::{
    Neo4jClient, Neo4jLongTermStore, Neo4jReasoningStore, Neo4jShortTermStore, SchemaManager,
};
use neo4rs::BoltType;

#[path = "common/fixture.rs"]
mod fixture;
use fixture::DockerFixture;

async fn graph_snapshot(client: &Neo4jClient) -> (Vec<BoltType>, Vec<BoltType>) {
    let nodes = client
        .execute(
            "MATCH (n) RETURN [elementId(n), labels(n), properties(n)] AS snapshot ORDER BY elementId(n)",
            vec![],
        )
        .await
        .unwrap();
    let relationships = client
        .execute(
            "MATCH (a)-[r]->(b) RETURN [elementId(r), elementId(a), elementId(b), type(r), properties(r)] AS snapshot ORDER BY elementId(r)",
            vec![],
        )
        .await
        .unwrap();
    let snapshot = |rows: Vec<neo4rs::Row>| {
        rows.iter()
            .map(|row| row.get::<BoltType>("snapshot").unwrap())
            .collect()
    };
    (snapshot(nodes), snapshot(relationships))
}

#[tokio::test]
async fn summary_entities_are_isolated_by_owner() {
    let Some(fixture) = DockerFixture::start("MUNINN_FIXTURE_URI") else {
        return;
    };
    let client = fixture.connect_fresh().await;
    SchemaManager::new(&client, 3).setup_all().await.unwrap();
    let store = Arc::new(Neo4jShortTermStore::new(client.clone()));
    let memory = MemoryClient::new(
        MemoryPorts {
            short_term: store.clone(),
            long_term: Arc::new(Neo4jLongTermStore::new(client.clone())),
            reasoning: Arc::new(Neo4jReasoningStore::new(client.clone())),
            embedder: None,
            cypher_query: None,
        },
        &MemoryOptions::default(),
    )
    .unwrap();
    let short_term = memory.short_term();
    let session = "synthetic-summary-session";
    let cases = [
        (Some("owner-a"), "Juniper"),
        (Some("owner-b"), "Cedar"),
        (None, "Willow"),
    ];

    for (owner, entity) in cases {
        short_term
            .add_message(
                session,
                MessageRole::User,
                entity,
                AddMessageOptions {
                    extraction_mode: Some(ExtractionMode::Explicit),
                    explicit_mentions: vec![EntityRef::NameType {
                        name: entity.into(),
                        entity_type: "OBJECT".into(),
                    }],
                    generate_embedding: false,
                    user_identifier: owner.map(str::to_owned),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }

    short_term
        .add_message(
            "synthetic-budget-session",
            MessageRole::User,
            "Earlier message",
            AddMessageOptions {
                generate_embedding: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    short_term
        .add_message(
            "synthetic-budget-session",
            MessageRole::Assistant,
            "Latest message",
            AddMessageOptions {
                generate_embedding: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let before = graph_snapshot(&client).await;

    for (owner, entity) in cases {
        let summary = short_term
            .get_conversation_summary(session, owner, 100, true, None::<fn(String) -> String>)
            .await
            .unwrap();
        assert_eq!(summary.message_count, 1);
        assert_eq!(summary.summary, format!("user: {entity}"));
        assert_eq!(summary.key_entities, [entity]);
    }

    let summary = short_term
        .get_conversation_summary(
            session,
            Some("owner-a"),
            100,
            false,
            None::<fn(String) -> String>,
        )
        .await
        .unwrap();
    assert!(summary.key_entities.is_empty());

    let empty = short_term
        .get_conversation_summary(
            "missing-session",
            None,
            100,
            true,
            None::<fn(String) -> String>,
        )
        .await
        .unwrap();
    assert_eq!(empty.message_count, 0);
    assert_eq!(empty.summary, "No messages in this conversation.");
    assert!(empty.key_entities.is_empty());

    let summarize_with_budget = |budget| {
        short_term.get_conversation_summary(
            "synthetic-budget-session",
            None,
            budget,
            false,
            None::<fn(String) -> String>,
        )
    };
    let short = summarize_with_budget(6).await.unwrap();
    let full = summarize_with_budget(100).await.unwrap();
    assert_eq!(short.message_count, 2);
    assert_eq!(short.summary, "assistant: Latest message");
    assert_eq!(
        full.summary,
        "user: Earlier message\nassistant: Latest message"
    );
    assert_eq!(
        graph_snapshot(&client).await,
        before,
        "summary must not persist graph changes"
    );

    // A legacy conversation with no owner_scope makes the session ambiguous.
    client
        .execute(
            "CREATE (:Conversation {session_id: $session_id})",
            vec![("session_id", session.into())],
        )
        .await
        .unwrap();
    let owner = OwnerScope::owned("owner-a").unwrap();
    let error = store.summary_entities(session, &owner).await.unwrap_err();
    assert!(error
        .to_string()
        .contains("ambiguous legacy conversation requires explicit migration"));
}
