//! Opt-in Bolt integration. Run only against a fresh, disposable Neo4j 5.26.
use std::sync::Arc;

use muninn_application::{MemoryClient, MemoryOptions, MemoryPorts, ToolCallOptions};
use muninn_core::model::{MessageRole, Metadata, ToolCallStatus};
use muninn_neo4j::{
    Neo4jClient, Neo4jConfig, Neo4jLongTermStore, Neo4jReasoningStore, Neo4jShortTermStore,
    SchemaManager,
};

const OWNER: &str = "synthetic-operator-owner";
const SESSION: &str = "sample-inspection";
const REQUEST: &str = "Inspect the sample";
const REPLY: &str = "The sample is ready";
const CATEGORY: &str = "reporting";
const PREFERENCE: &str = "Use concise reports";
const TASK: &str = "Summarize the synthetic sample";
const TOOL: &str = "sample_lookup";

fn memory(client: &Neo4jClient) -> MemoryClient {
    MemoryClient::new(
        MemoryPorts {
            short_term: Arc::new(Neo4jShortTermStore::new(client.clone())),
            long_term: Arc::new(Neo4jLongTermStore::new(client.clone())),
            reasoning: Arc::new(Neo4jReasoningStore::new(client.clone())),
            embedder: None,
            cypher_query: None,
        },
        &MemoryOptions::default(),
    )
    .unwrap()
    .with_local_operator(OWNER)
    .unwrap()
}

#[tokio::test]
#[ignore = "requires a fresh authenticated loopback Neo4j 5.26; caller owns teardown"]
async fn shared_local_operator_round_trips_over_bolt() {
    assert_eq!(
        std::env::var("MUNINN_LOCAL_OPERATOR_FRESH_SYNTHETIC").as_deref(),
        Ok("1"),
        "explicit disposable fixture confirmation required"
    );
    let uri = std::env::var("MUNINN_FIXTURE_URI").expect("fixture URI required");
    assert!(
        uri.strip_prefix("bolt://127.0.0.1:")
            .and_then(|port| port.parse::<u16>().ok())
            .is_some_and(|port| port != 0),
        "fixture must use IPv4 loopback Bolt"
    );
    let password = std::env::var("MUNINN_FIXTURE_PASSWORD").expect("fixture password required");
    assert!(!password.is_empty(), "fixture password must not be empty");
    let config = Neo4jConfig {
        uri,
        username: "neo4j".into(),
        password,
        database: Some("neo4j".into()),
        ..Default::default()
    };
    let client = Neo4jClient::connect(&config).await.unwrap();
    let rows = client
        .execute(
            "CALL dbms.components() YIELD versions RETURN versions[0] AS version",
            vec![],
        )
        .await
        .unwrap();
    let version: String = rows[0].get("version").unwrap();
    assert!(
        version.starts_with("5.26."),
        "unexpected Neo4j version: {version}"
    );
    let rows = client
        .execute("MATCH (n) RETURN count(n) AS nodes", vec![])
        .await
        .unwrap();
    assert_eq!(
        rows[0].get::<i64>("nodes").unwrap(),
        0,
        "fixture must be empty"
    );
    eprintln!("Neo4j {version}: fixture starts with zero nodes");
    SchemaManager::new(&client, 3).setup_all().await.unwrap();

    let writer = memory(&client);
    let view = writer.local_operator();
    let request = view
        .short_term()
        .client()
        .unwrap()
        .add_message(SESSION, MessageRole::User, REQUEST)
        .await
        .unwrap();
    let reply = writer
        .graph()
        .unwrap()
        .add_message(SESSION, MessageRole::Assistant, REPLY)
        .await
        .unwrap();
    let preference = view
        .long_term()
        .client()
        .unwrap()
        .add_preference(CATEGORY, PREFERENCE)
        .await
        .unwrap();
    let conversation_id = request
        .conversation_id
        .expect("message belongs to conversation");
    assert_eq!(reply.conversation_id, Some(conversation_id));

    // The operator can read hosted steps and calls, but cannot create them.
    // Seed one of each through the trusted application API before starting a standalone trace.
    let step = writer
        .reasoning()
        .record_conversation_step(
            conversation_id,
            Some(OWNER),
            "Check the sample",
            "Read status",
            Some("ready"),
        )
        .await
        .unwrap();
    let call = writer
        .reasoning()
        .record_tool_call(
            step.id,
            TOOL,
            Metadata::new(),
            ToolCallOptions {
                user_identifier: Some(OWNER.into()),
                status: ToolCallStatus::Success,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let standalone = view
        .reasoning()
        .client()
        .unwrap()
        .start_trace(SESSION, TASK)
        .await
        .unwrap();
    drop(writer);
    drop(client);

    // Read with a fresh Bolt pool; each entry point must see the same persisted data.
    let client = Neo4jClient::connect(&config).await.unwrap();
    let reader = memory(&client);
    let view = reader.local_operator();
    for operator in [
        reader.graph().unwrap(),
        view.short_term().client().unwrap(),
        view.long_term().client().unwrap(),
        view.reasoning().client().unwrap(),
    ] {
        let conversation = operator.get_conversation(SESSION, Some(20)).await.unwrap();
        assert_eq!(conversation.id, conversation_id);
        assert_eq!(conversation.session_id, SESSION);
        assert_eq!(conversation.user_identifier.as_deref(), Some(OWNER));
        let messages: Vec<_> = conversation
            .messages
            .iter()
            .map(|message| {
                assert!(message.embedding.is_none());
                (message.id, message.role, message.content.as_str())
            })
            .collect();
        assert_eq!(
            messages,
            [
                (request.id, MessageRole::User, REQUEST),
                (reply.id, MessageRole::Assistant, REPLY)
            ]
        );

        let preferences = operator.get_preferences(true).await.unwrap();
        assert_eq!(preferences.len(), 1);
        assert_eq!(preferences[0].id, preference.id);
        assert_eq!(preferences[0].category, CATEGORY);
        assert_eq!(preferences[0].preference, PREFERENCE);
        assert!(preferences[0].embedding.is_none());

        let hosted = operator
            .get_conversation_trace(conversation_id)
            .await
            .unwrap();
        assert_eq!(hosted.conversation_id, conversation_id);
        assert_eq!(hosted.steps.as_slice(), std::slice::from_ref(&step));
        assert_eq!(hosted.tool_calls.len(), 1);
        assert_eq!(hosted.tool_calls[0].id, call.id);
        assert_eq!(hosted.tool_calls[0].step_id, Some(step.id));
        assert_eq!(hosted.tool_calls[0].tool_name, TOOL);
        assert_eq!(hosted.tool_calls[0].status, ToolCallStatus::Success);
    }

    // The scoped read API intentionally cannot list standalone traces. Check the
    // persisted owner relationship and the lack of conversation/message links directly.
    let rows = client
        .execute(
            "MATCH (:User {identifier: $owner})-[:HAS_TRACE]->(t:ReasoningTrace {id: $trace})
         RETURN t.user_identifier AS owner, t.session_id AS session, t.task AS task,
                t.task_embedding IS NULL AS no_embedding,
                NOT EXISTS { MATCH (:Conversation)-[:HAS_TRACE]->(t) } AS standalone,
                NOT EXISTS { MATCH (t)-[:INITIATED_BY]->(:Message) } AS no_message",
            vec![
                ("owner", OWNER.into()),
                ("trace", standalone.id.to_string().into()),
            ],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get::<String>("owner").unwrap(), OWNER);
    assert_eq!(rows[0].get::<String>("session").unwrap(), SESSION);
    assert_eq!(rows[0].get::<String>("task").unwrap(), TASK);
    for flag in ["no_embedding", "standalone", "no_message"] {
        assert!(
            rows[0].get::<bool>(flag).unwrap(),
            "trace must satisfy {flag}"
        );
    }
}
