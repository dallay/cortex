//! Opt-in, non-destructive H1 reasoning adapter checks. Requires a NEW isolated
//! authenticated fixture, never the active Muninn database.
use muninn_core::model::{Metadata, ToolCall, ToolCallStatus};
use muninn_core::ports::{OwnerScope, ReasoningStore, StepRecord, TraceRecord};
use muninn_neo4j::{Neo4jClient, Neo4jConfig, Neo4jReasoningStore};
use uuid::Uuid;

fn trace() -> TraceRecord {
    TraceRecord {
        id: Uuid::new_v4(),
        session_id: "shared".into(),
        task: "H1".into(),
        task_embedding: None,
        metadata: Metadata::new(),
    }
}
fn step() -> StepRecord {
    StepRecord {
        id: Uuid::new_v4(),
        thought: Some("approved summary".into()),
        action: None,
        observation: None,
        embedding: None,
        metadata: Metadata::new(),
    }
}

#[tokio::test]
async fn reasoning_owner_scope_real_adapter() {
    if std::env::var("MUNINN_REASONING_ISOLATED_TEST").as_deref() != Ok("1") {
        eprintln!(
            "SKIP: set MUNINN_REASONING_ISOLATED_TEST=1 for a NEW isolated authenticated fixture"
        );
        return;
    }
    let client = Neo4jClient::connect(&Neo4jConfig {
        uri: std::env::var("MUNINN_REASONING_TEST_URI").expect("isolated URI required"),
        username: "neo4j".into(),
        password: std::env::var("MUNINN_REASONING_TEST_PASSWORD")
            .expect("fixture password required"),
        ..Default::default()
    })
    .await
    .unwrap();
    let store = Neo4jReasoningStore::new(client.clone());
    let tag = Uuid::new_v4().to_string();
    let scopes = [
        OwnerScope::owned(&format!("A-{tag}")).unwrap(),
        OwnerScope::owned(&format!("B-{tag}")).unwrap(),
        OwnerScope::unowned(),
    ];
    let mut conversations = Vec::new();
    let mut messages = Vec::new();
    for scope in &scopes {
        let c = Uuid::new_v4();
        let m = Uuid::new_v4();
        client.execute("CREATE (c:Conversation {id: $c, owner_scope: $scope, session_id: $session}) CREATE (c)-[:HAS_MESSAGE]->(:Message {id: $m})", vec![("c", c.to_string().into()), ("m", m.to_string().into()), ("scope", scope.storage_key().into()), ("session", tag.clone().into())]).await.unwrap();
        conversations.push(c);
        messages.push(m);
    }
    // Before any trace exists, foreign UUIDs must not create traces or steps.
    for (i, scope) in scopes.iter().enumerate() {
        for j in 0..3 {
            if i == j {
                continue;
            }
            assert!(store
                .open_conversation_trace(conversations[j], scope)
                .await
                .unwrap()
                .is_none());
            assert!(store
                .conversation_steps(conversations[j], scope)
                .await
                .unwrap()
                .is_none());
            assert!(store
                .ensure_conversation_trace(conversations[j], &trace(), scope)
                .await
                .is_err());
            assert!(store
                .validate_message_scope(messages[j], scope)
                .await
                .is_err());
            let record = trace();
            assert!(store
                .create_trace(&record, scope, Some(messages[j]))
                .await
                .is_err());
            assert!(store
                .get_trace_with_steps(record.id)
                .await
                .unwrap()
                .is_none());
        }
    }
    let mut traces = Vec::new();
    let mut calls = Vec::new();
    for (i, scope) in scopes.iter().enumerate() {
        let id = store
            .ensure_conversation_trace(conversations[i], &trace(), scope)
            .await
            .unwrap();
        let st = store
            .add_conversation_step(conversations[i], id, scope, &step())
            .await
            .unwrap();
        assert_eq!(
            store
                .conversation_steps(conversations[i], scope)
                .await
                .unwrap()
                .unwrap()
                .len(),
            1
        );
        let call = ToolCall {
            id: Uuid::new_v4(),
            step_id: Some(st.id),
            tool_name: format!("H1-{tag}"),
            arguments: Metadata::new(),
            result: None,
            status: ToolCallStatus::Success,
            duration_ms: None,
            error: None,
            created_at: chrono::Utc::now(),
        };
        store
            .record_tool_call(&call, scope, Some(messages[i]))
            .await
            .unwrap();
        assert!(store
            .link_trace_to_message(id, messages[i], scope)
            .await
            .unwrap());
        assert!(store
            .link_conversation_to_trace(conversations[i], id, scope)
            .await
            .unwrap());
        assert!(store
            .link_tool_call_to_message(call.id, messages[i], scope)
            .await
            .unwrap());
        traces.push((id, st.id));
        calls.push(call);
    }
    // Hosted context reads must never return another owner's conversation.
    let entity_id = Uuid::new_v4().to_string();
    client.execute("CREATE (e:Entity {id: $entity}) WITH e MATCH (s:ReasoningStep) WHERE s.id IN $steps CREATE (s)-[:TOUCHED]->(e)",
        vec![("entity", entity_id.clone().into()),
             ("steps", traces.iter().map(|(_, id)| id.to_string()).collect::<Vec<_>>().into())]).await.unwrap();
    for (i, scope) in scopes.iter().enumerate() {
        for (j, (_, step_id)) in traces.iter().enumerate() {
            let context = store.step_context(*step_id, scope).await.unwrap();
            assert_eq!(context.is_some(), i == j);
            if let Some(context) = context {
                assert_eq!(context.conversation_id, Some(conversations[i]));
            }
        }
        let provenance = store.entity_provenance(&entity_id, scope).await.unwrap();
        assert_eq!(provenance.len(), 1);
        assert_eq!(provenance[0].conversation_id, conversations[i]);
    }

    // Exercise the post-check rollback, without any concurrent writer.
    let rolled_back_legacy = Uuid::new_v4();
    assert!(matches!(
        client
            .execute_guarded(
                muninn_neo4j::queries::reasoning::LEGACY_CONFLICT,
                vec![
                    ("guard_conversation", conversations[0].to_string().into()),
                    ("guard_message", neo4rs::BoltType::Null(neo4rs::BoltNull)),
                    ("guard_trace", neo4rs::BoltType::Null(neo4rs::BoltNull)),
                    ("guard_step", neo4rs::BoltType::Null(neo4rs::BoltNull)),
                    ("guard_call", neo4rs::BoltType::Null(neo4rs::BoltNull)),
                    ("guard_entity", neo4rs::BoltType::Null(neo4rs::BoltNull)),
                    ("guard_owner", scopes[0].storage_key().into())
                ],
                "CREATE (c:Conversation {id: $id, session_id: $session}) RETURN c",
                vec![
                    ("id", rolled_back_legacy.to_string().into()),
                    ("session", tag.clone().into())
                ],
            )
            .await,
        Err(muninn_core::MemoryError::InvalidInput(_))
    ));
    assert!(client
        .execute(
            "MATCH (c:Conversation {id: $id}) RETURN c",
            vec![("id", rolled_back_legacy.to_string().into())]
        )
        .await
        .unwrap()
        .is_empty());

    let legacy_id = Uuid::new_v4();
    client
        .execute(
            "CREATE (:Conversation {id: $id, session_id: $session_id})",
            vec![
                ("id", legacy_id.to_string().into()),
                ("session_id", tag.clone().into()),
            ],
        )
        .await
        .unwrap();
    assert!(matches!(
        store.conversation_steps(conversations[0], &scopes[0]).await,
        Err(muninn_core::MemoryError::InvalidInput(_))
    ));
    assert!(matches!(
        store.step_context(traces[0].1, &scopes[0]).await,
        Err(muninn_core::MemoryError::InvalidInput(_))
    ));
    assert!(matches!(
        store.entity_provenance(&entity_id, &scopes[0]).await,
        Err(muninn_core::MemoryError::InvalidInput(_))
    ));
    let rejected_trace = trace();
    assert!(matches!(
        store
            .create_trace(&rejected_trace, &scopes[0], Some(messages[0]))
            .await,
        Err(muninn_core::MemoryError::InvalidInput(_))
    ));
    assert!(client
        .execute(
            "MATCH (rt:ReasoningTrace {id: $id}) RETURN rt",
            vec![("id", rejected_trace.id.to_string().into())]
        )
        .await
        .unwrap()
        .is_empty());
    let rejected_step = step();
    assert!(matches!(
        store
            .add_conversation_step(conversations[0], traces[0].0, &scopes[0], &rejected_step)
            .await,
        Err(muninn_core::MemoryError::InvalidInput(_))
    ));
    assert!(client
        .execute(
            "MATCH (s:ReasoningStep {id: $id}) RETURN s",
            vec![("id", rejected_step.id.to_string().into())]
        )
        .await
        .unwrap()
        .is_empty());
    client
        .execute(
            "MATCH (c:Conversation {id: $id}) DELETE c",
            vec![("id", legacy_id.to_string().into())],
        )
        .await
        .unwrap();

    for (i, scope) in scopes.iter().enumerate() {
        for j in 0..3 {
            if i == j {
                continue;
            }
            assert!(store
                .open_conversation_trace(conversations[j], scope)
                .await
                .unwrap()
                .is_none());
            assert!(store
                .conversation_steps(conversations[j], scope)
                .await
                .unwrap()
                .is_none());
            assert!(store
                .add_conversation_step(conversations[j], traces[j].0, scope, &step())
                .await
                .is_err());
            assert!(store
                .ensure_conversation_trace(conversations[j], &trace(), scope)
                .await
                .is_err());
            assert!(!store
                .link_trace_to_message(traces[i].0, messages[j], scope)
                .await
                .unwrap());
            assert!(!store
                .link_trace_to_message(traces[j].0, messages[i], scope)
                .await
                .unwrap());
            assert!(!store
                .link_conversation_to_trace(conversations[j], traces[i].0, scope)
                .await
                .unwrap());
            assert!(!store
                .link_conversation_to_trace(conversations[i], traces[j].0, scope)
                .await
                .unwrap());
            assert!(!store
                .link_tool_call_to_message(calls[i].id, messages[j], scope)
                .await
                .unwrap());
            assert!(!store
                .link_tool_call_to_message(calls[j].id, messages[i], scope)
                .await
                .unwrap());
            let mut bad = calls[i].clone();
            bad.id = Uuid::new_v4();
            assert!(store
                .record_tool_call(&bad, scope, Some(messages[j]))
                .await
                .is_err());
            assert_eq!(
                store
                    .get_trace_with_steps(traces[i].0)
                    .await
                    .unwrap()
                    .unwrap()
                    .steps[0]
                    .tool_calls
                    .len(),
                1
            );
        }
    }
    // Legacy conversation without owner_scope remains inaccessible, including unowned.
    let legacy = Uuid::new_v4();
    client
        .execute(
            "CREATE (:Conversation {id: $id, session_id: $session})",
            vec![("id", legacy.to_string().into()), ("session", tag.into())],
        )
        .await
        .unwrap();
    for scope in &scopes {
        assert!(store
            .ensure_conversation_trace(legacy, &trace(), scope)
            .await
            .is_err());
        assert!(matches!(
            store.conversation_steps(legacy, scope).await,
            Err(muninn_core::MemoryError::InvalidInput(_))
        ));
    }
}
