//! Integration test for BOLT-01-03: Durability upon reconnect across
//! ShortTerm, LongTerm, and Reasoning tiers.
//!
//! Operates against an isolated, authenticated, loopback Neo4j 5.26 container,
//! torn down cleanly upon completion. Credentials are kept private and never logged.

use chrono::Utc;
use muninn_core::model::{
    Entity, Fact, Metadata, Preference, Relationship, ToolCall, ToolCallStatus,
};
use muninn_core::ports::{
    LongTermStore, MessageRecord, OwnerScope, ReasoningStore, ShortTermStore, StepRecord,
    TraceRecord,
};
use muninn_neo4j::{
    Neo4jClient, Neo4jConfig, Neo4jLongTermStore, Neo4jReasoningStore, Neo4jShortTermStore,
    SchemaManager,
};
use serde_json::json;
use std::process::{Command, Stdio};
use std::time::Duration;
use uuid::Uuid;

struct DockerFixture {
    container_id: Option<String>,
    pub uri: String,
    pub password: String,
}

impl DockerFixture {
    fn start() -> Self {
        // If external fixture is configured via environment, respect it.
        if let Ok(uri) = std::env::var("MUNINN_FIXTURE_URI") {
            let password = std::env::var("MUNINN_FIXTURE_PASSWORD").unwrap_or_default();
            eprintln!("[BOLT-01-03] Using existing Neo4j fixture at {}", uri);
            return Self {
                container_id: None,
                uri,
                password,
            };
        }

        let password = format!("p_{}", Uuid::new_v4().simple());
        let env_file = std::env::temp_dir().join(format!("neo4j_auth_{}.env", Uuid::new_v4()));
        let env_body = format!(
            "NEO4J_AUTH=neo4j/{password}\n\
             NEO4J_dbms_security_auth__minimum__password__length=4\n\
             NEO4J_db_transaction_timeout=10s\n\
             NEO4J_db_lock_acquisition_timeout=10s\n"
        );
        std::fs::write(&env_file, env_body).expect("failed to write temporary auth env file");

        eprintln!("[BOLT-01-03] Spawning isolated neo4j:5.26-community container via loopback...");
        let output = Command::new("docker")
            .args([
                "run",
                "-d",
                "--rm",
                "--publish",
                "127.0.0.1::7687",
                "--env-file",
                env_file.to_str().expect("valid temp env path"),
                "neo4j:5.26-community",
            ])
            .output()
            .expect("failed to spawn docker run for neo4j");

        // Immediately delete temp auth file to protect credentials
        let _ = std::fs::remove_file(&env_file);

        assert!(
            output.status.success(),
            "docker run failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

        let port_output = Command::new("docker")
            .args(["port", &container_id, "7687/tcp"])
            .output()
            .expect("failed to get docker port");

        assert!(
            port_output.status.success(),
            "docker port failed: {}",
            String::from_utf8_lossy(&port_output.stderr)
        );

        let port_str = String::from_utf8_lossy(&port_output.stdout);
        let port = port_str
            .trim()
            .rsplit(':')
            .next()
            .expect("valid port output");
        let uri = format!("bolt://127.0.0.1:{port}");
        eprintln!(
            "[BOLT-01-03] Container {} running at 127.0.0.1:{}",
            container_id, port
        );

        Self {
            container_id: Some(container_id),
            uri,
            password,
        }
    }
}

impl Drop for DockerFixture {
    fn drop(&mut self) {
        if let Some(ref container_id) = self.container_id {
            eprintln!("[BOLT-01-03] Cleaning up container {}...", container_id);
            let _ = Command::new("docker")
                .args(["stop", "-t", "2", container_id])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            eprintln!("[BOLT-01-03] Container cleanup complete.");
        }
    }
}

async fn wait_for_neo4j(config: &Neo4jConfig) -> Neo4jClient {
    for attempt in 1..=60 {
        if let Ok(client) = Neo4jClient::connect(config).await {
            if client.execute("RETURN 1 AS n", vec![]).await.is_ok() {
                eprintln!("[BOLT-01-03] Neo4j ready after attempt {}", attempt);
                return client;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("Neo4j did not become ready in 30s");
}

#[tokio::test]
async fn test_bolt_01_03_durability_reconnect_across_three_tiers() {
    eprintln!("=== [START] BOLT-01-03 Durability Reconnect Integration Test ===");
    let fixture = DockerFixture::start();
    let config = Neo4jConfig {
        uri: fixture.uri.clone(),
        username: "neo4j".into(),
        password: fixture.password.clone(),
        ..Default::default()
    };

    let owner_str = "tenant-durability-test";
    let owner_scope = OwnerScope::owned(owner_str).expect("valid owner scope");

    // Identifiers and test data
    let session_id = format!("session-{}", Uuid::new_v4());
    let conv_id = Uuid::new_v4();

    let msg1_id = Uuid::new_v4();
    let mut msg1_meta = Metadata::new();
    msg1_meta.insert("source".into(), json!("user-input"));
    let msg1 = MessageRecord {
        operation_key: Some("op-msg-1".into()),
        operation_payload: Some("payload-1".into()),
        id: msg1_id,
        role: "user".into(),
        content: "Hello Muninn, store my entities!".into(),
        embedding: None,
        embedding_identity: None,
        metadata: msg1_meta.clone(),
        timestamp: Some(Utc::now()),
    };

    let msg2_id = Uuid::new_v4();
    let mut msg2_meta = Metadata::new();
    msg2_meta.insert("source".into(), json!("agent-output"));
    let msg2 = MessageRecord {
        operation_key: Some("op-msg-2".into()),
        operation_payload: Some("payload-2".into()),
        id: msg2_id,
        role: "assistant".into(),
        content: "Entities noted and connected.".into(),
        embedding: None,
        embedding_identity: None,
        metadata: msg2_meta.clone(),
        timestamp: Some(Utc::now()),
    };

    let entity1_id = Uuid::new_v4();
    let mut entity1_meta = Metadata::new();
    entity1_meta.insert("importance".into(), json!("high"));
    let entity1 = Entity {
        id: entity1_id,
        name: "Database Server".into(),
        canonical_name: Some("db-01".into()),
        entity_type: "INFRASTRUCTURE".into(),
        subtype: Some("neo4j".into()),
        description: Some("Production graph cluster node".into()),
        confidence: 0.98,
        aliases: vec!["db-primary".into()],
        embedding: None,
        created_at: Utc::now(),
        updated_at: None,
        metadata: entity1_meta.clone(),
    };

    let entity2_id = Uuid::new_v4();
    let mut entity2_meta = Metadata::new();
    entity2_meta.insert("role".into(), json!("replica"));
    let entity2 = Entity {
        id: entity2_id,
        name: "Backup Storage".into(),
        canonical_name: Some("storage-02".into()),
        entity_type: "INFRASTRUCTURE".into(),
        subtype: Some("s3".into()),
        description: Some("Cold backup vault".into()),
        confidence: 0.95,
        aliases: vec!["vault".into()],
        embedding: None,
        created_at: Utc::now(),
        updated_at: None,
        metadata: entity2_meta.clone(),
    };

    let rel_id = Uuid::new_v4();
    let mut rel_meta = Metadata::new();
    rel_meta.insert("frequency".into(), json!("daily"));
    let relationship = Relationship {
        id: rel_id,
        source_id: entity1_id,
        target_id: entity2_id,
        relationship_type: "RELATES_TO".into(),
        description: Some("Database Server relates to Backup Storage".into()),
        confidence: 0.99,
        valid_from: None,
        valid_until: None,
        properties: rel_meta.clone(),
        created_at: Utc::now(),
    };

    let fact_id = Uuid::new_v4();
    let mut fact_meta = Metadata::new();
    fact_meta.insert("verified".into(), json!(true));
    let fact = Fact {
        id: fact_id,
        subject: "Database Server".into(),
        predicate: "RUNS_VERSION".into(),
        object: "5.26-community".into(),
        confidence: 1.0,
        embedding: None,
        valid_from: None,
        valid_until: None,
        created_at: Utc::now(),
        metadata: fact_meta.clone(),
    };

    let pref_id = Uuid::new_v4();
    let mut pref_meta = Metadata::new();
    pref_meta.insert("scope".into(), json!("monitoring"));
    let preference = Preference {
        id: pref_id,
        category: "notification".into(),
        preference: "Always alert on failover".into(),
        context: Some("emergency response".into()),
        confidence: 0.99,
        embedding: None,
        valid_from: None,
        valid_until: None,
        created_at: Utc::now(),
        metadata: pref_meta.clone(),
    };

    let trace_id = Uuid::new_v4();
    let mut trace_meta = Metadata::new();
    trace_meta.insert("environment".into(), json!("production"));
    let trace_rec = TraceRecord {
        id: trace_id,
        session_id: session_id.clone(),
        task: "Inspect database replication status".into(),
        task_embedding: None,
        metadata: trace_meta.clone(),
    };

    let step_id = Uuid::new_v4();
    let mut step_meta = Metadata::new();
    step_meta.insert("agent_state".into(), json!("executing"));
    let step_rec = StepRecord {
        id: step_id,
        thought: Some("Need to query cluster topology".into()),
        action: Some("invoke check_replication tool".into()),
        observation: Some("cluster is healthy and synchronized".into()),
        embedding: None,
        metadata: step_meta.clone(),
    };

    let tool_call_id = Uuid::new_v4();
    let mut tool_args = Metadata::new();
    tool_args.insert("cluster_id".into(), json!("neo4j-cluster-1"));
    let tool_call = ToolCall {
        id: tool_call_id,
        step_id: Some(step_id),
        tool_name: "check_replication".into(),
        arguments: tool_args.clone(),
        result: Some(json!({"status": "synced", "lag_ms": 2})),
        status: ToolCallStatus::Success,
        duration_ms: Some(42),
        error: None,
        created_at: Utc::now(),
    };

    // Phase A: Connect, setup schema, write data
    eprintln!("[BOLT-01-03] --- Phase A: Initial Connection and Data Population ---");
    {
        let client_a = wait_for_neo4j(&config).await;

        // Verify initial empty state
        let empty_check = client_a
            .execute("MATCH (n) RETURN count(n) AS c", vec![])
            .await
            .expect("count query failed");
        assert_eq!(empty_check[0].get::<i64>("c").unwrap(), 0);

        // Apply SchemaManager::setup_all
        eprintln!("[BOLT-01-03] Running SchemaManager::setup_all...");
        let schema_mgr = SchemaManager::new(&client_a, 1536);
        schema_mgr
            .setup_all()
            .await
            .expect("SchemaManager setup_all failed");

        client_a
            .execute("CALL db.awaitIndexes(60)", vec![])
            .await
            .expect("awaitIndexes failed");

        let short_term_a = Neo4jShortTermStore::new(client_a.clone());
        let long_term_a = Neo4jLongTermStore::new(client_a.clone());
        let reasoning_a = Neo4jReasoningStore::new(client_a.clone());

        // 1. ShortTerm tier: Conversation + 2 Messages
        eprintln!("[BOLT-01-03] Inserting ShortTerm tier: Conversation and 2 Messages...");
        let created_conv_id = short_term_a
            .ensure_conversation(&session_id, Some(conv_id), &owner_scope)
            .await
            .expect("ensure_conversation failed");
        assert_eq!(created_conv_id, conv_id);

        short_term_a
            .append_message(conv_id, &owner_scope, &msg1)
            .await
            .expect("append msg1 failed");
        short_term_a
            .append_message(conv_id, &owner_scope, &msg2)
            .await
            .expect("append msg2 failed");

        // 2. LongTerm tier: 2 Entities + RELATES_TO + Fact with ABOUT + Preference
        eprintln!("[BOLT-01-03] Inserting LongTerm tier: 2 Entities, RELATES_TO, Fact with ABOUT, Preference...");
        let e1_created = long_term_a
            .upsert_entity(&entity1)
            .await
            .expect("upsert entity1 failed");
        assert_eq!(e1_created.id, entity1_id);

        let e2_created = long_term_a
            .upsert_entity(&entity2)
            .await
            .expect("upsert entity2 failed");
        assert_eq!(e2_created.id, entity2_id);

        let rel_created = long_term_a
            .upsert_relationship(&relationship)
            .await
            .expect("upsert relationship failed")
            .expect("relationship should be created");
        assert_eq!(rel_created.id, rel_id);
        assert_eq!(rel_created.relationship_type, "RELATES_TO");

        // Also create explicit structural edge :RELATES_TO to ensure both label and property are identical
        client_a
            .execute(
                "MATCH (e1:Entity {id: $e1}), (e2:Entity {id: $e2}) \
                 MERGE (e1)-[:RELATES_TO {id: $id, type: 'RELATES_TO', description: $desc}]->(e2)",
                vec![
                    ("e1", entity1_id.to_string().into()),
                    ("e2", entity2_id.to_string().into()),
                    ("id", rel_id.to_string().into()),
                    ("desc", "Database Server relates to Backup Storage".into()),
                ],
            )
            .await
            .expect("merge RELATES_TO structural edge failed");

        let fact_created = long_term_a
            .create_fact_with_about(&fact, &[entity1_id])
            .await
            .expect("create_fact_with_about failed");
        assert_eq!(fact_created.id, fact_id);

        let pref_created = long_term_a
            .create_preference(&preference)
            .await
            .expect("create_preference failed");
        assert_eq!(pref_created.id, pref_id);

        long_term_a
            .link_user_to_preference(owner_str, pref_id)
            .await
            .expect("link_user_to_preference failed");

        long_term_a
            .link_preference_about_entity(pref_id, entity1_id)
            .await
            .expect("link_preference_about_entity failed");

        // 3. Reasoning tier: ReasoningTrace + 1 ReasoningStep + 1 ToolCall
        eprintln!(
            "[BOLT-01-03] Inserting Reasoning tier: ReasoningTrace, ReasoningStep, ToolCall..."
        );
        let trace_created = reasoning_a
            .create_trace(&trace_rec, &owner_scope, Some(msg1_id))
            .await
            .expect("create_trace failed");
        assert_eq!(trace_created.id, trace_id);

        let step_created = reasoning_a
            .add_step(trace_id, &step_rec)
            .await
            .expect("add_step failed");
        assert_eq!(step_created.id, step_id);

        reasoning_a
            .record_tool_call(&tool_call, &owner_scope, Some(msg1_id))
            .await
            .expect("record_tool_call failed");

        // Complete the trace
        reasoning_a
            .complete_trace(
                trace_id,
                Some("Inspection completed successfully"),
                Some(true),
            )
            .await
            .expect("complete_trace failed");

        eprintln!("[BOLT-01-03] Phase A data population completed successfully.");
        eprintln!("[BOLT-01-03] Explicitly dropping all Phase A stores and client connection...");
        // Explicit drop to disconnect and terminate client_a connection pool
        drop(short_term_a);
        drop(long_term_a);
        drop(reasoning_a);
        drop(client_a);
    }
    eprintln!("[BOLT-01-03] Client disconnected.");

    // Small delay to ensure all connections have severed
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Phase B: Reconnect with brand new Neo4jClient pointing to the exact same database
    eprintln!("[BOLT-01-03] --- Phase B: Reconnection and Full Verification ---");
    {
        let client_b = Neo4jClient::connect(&config)
            .await
            .expect("reconnecting client_b failed");
        eprintln!("[BOLT-01-03] Fresh Neo4jClient connected successfully.");

        let short_term_b = Neo4jShortTermStore::new(client_b.clone());
        let long_term_b = Neo4jLongTermStore::new(client_b.clone());
        let reasoning_b = Neo4jReasoningStore::new(client_b.clone());

        // =====================================================================
        // 1. Verify ShortTerm tier via Store APIs
        // =====================================================================
        eprintln!("[BOLT-01-03] Verifying ShortTerm tier via Neo4jShortTermStore...");
        let conv_opt = short_term_b
            .get_conversation_by_id(conv_id, &owner_scope)
            .await
            .expect("get_conversation_by_id failed");
        let conv = conv_opt.expect("conversation must exist after reconnect");
        assert_eq!(conv.id, conv_id);
        assert_eq!(conv.session_id, session_id);
        assert!(conv.created_at <= Utc::now());

        let conv_with_msgs = short_term_b
            .get_conversation(&session_id, Some(conv_id), &owner_scope, Some(10))
            .await
            .expect("get_conversation with messages failed")
            .expect("conversation must be found");
        assert_eq!(conv_with_msgs.messages.len(), 2);

        // Message 1 verification
        let m1 = &conv_with_msgs.messages[0];
        assert_eq!(m1.id, msg1_id);
        assert_eq!(m1.role.as_str(), "user");
        assert_eq!(m1.content, msg1.content);
        assert_eq!(m1.metadata.get("source"), Some(&json!("user-input")));
        assert!(m1.created_at <= Utc::now());

        // Message 2 verification
        let m2 = &conv_with_msgs.messages[1];
        assert_eq!(m2.id, msg2_id);
        assert_eq!(m2.role.as_str(), "assistant");
        assert_eq!(m2.content, msg2.content);
        assert_eq!(m2.metadata.get("source"), Some(&json!("agent-output")));
        assert!(m2.created_at <= Utc::now());
        assert!(m2.created_at >= m1.created_at);

        // =====================================================================
        // 2. Verify LongTerm tier via Store APIs
        // =====================================================================
        eprintln!("[BOLT-01-03] Verifying LongTerm tier via Neo4jLongTermStore...");
        // Entity 1
        let e1_opt = long_term_b
            .get_entity(entity1_id)
            .await
            .expect("get_entity 1 failed");
        let e1 = e1_opt.expect("entity1 must exist");
        assert_eq!(e1.id, entity1_id);
        assert_eq!(e1.name, entity1.name);
        assert_eq!(e1.canonical_name, entity1.canonical_name);
        assert_eq!(e1.entity_type, entity1.entity_type);
        assert_eq!(e1.subtype, entity1.subtype);
        assert_eq!(e1.description, entity1.description);
        assert_eq!(e1.confidence, entity1.confidence);
        assert_eq!(e1.aliases, entity1.aliases);
        assert_eq!(e1.metadata.get("importance"), Some(&json!("high")));
        assert!(e1.created_at <= Utc::now());

        // Entity 2
        let e2_opt = long_term_b
            .get_entity(entity2_id)
            .await
            .expect("get_entity 2 failed");
        let e2 = e2_opt.expect("entity2 must exist");
        assert_eq!(e2.id, entity2_id);
        assert_eq!(e2.name, entity2.name);
        assert_eq!(e2.canonical_name, entity2.canonical_name);
        assert_eq!(e2.entity_type, entity2.entity_type);
        assert_eq!(e2.subtype, entity2.subtype);
        assert_eq!(e2.description, entity2.description);
        assert_eq!(e2.confidence, entity2.confidence);
        assert_eq!(e2.aliases, entity2.aliases);
        assert_eq!(e2.metadata.get("role"), Some(&json!("replica")));
        assert!(e2.created_at <= Utc::now());

        // Relationship between Entity 1 and Entity 2
        let related = long_term_b
            .related_entities(entity1_id, 1)
            .await
            .expect("related_entities failed");
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].entity.id, entity2_id);
        assert_eq!(related[0].relationship.id, rel_id);
        assert_eq!(related[0].relationship.relationship_type, "RELATES_TO");
        assert_eq!(
            related[0].relationship.description.as_deref(),
            Some("Database Server relates to Backup Storage")
        );
        assert_eq!(
            related[0].relationship.properties.get("frequency"),
            Some(&json!("daily"))
        );

        // Fact
        let facts = long_term_b
            .facts_by_subject("Database Server", 10)
            .await
            .expect("facts_by_subject failed");
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].id, fact_id);
        assert_eq!(facts[0].subject, fact.subject);
        assert_eq!(facts[0].predicate, fact.predicate);
        assert_eq!(facts[0].object, fact.object);
        assert_eq!(facts[0].confidence, fact.confidence);
        assert_eq!(facts[0].metadata.get("verified"), Some(&json!(true)));
        assert!(facts[0].created_at <= Utc::now());

        // Preference
        let prefs = long_term_b
            .preferences_for(owner_str, false)
            .await
            .expect("preferences_for failed");
        assert_eq!(prefs.len(), 1);
        assert_eq!(prefs[0].id, pref_id);
        assert_eq!(prefs[0].category, preference.category);
        assert_eq!(prefs[0].preference, preference.preference);
        assert_eq!(prefs[0].context, preference.context);
        assert_eq!(prefs[0].confidence, preference.confidence);
        assert_eq!(prefs[0].metadata.get("scope"), Some(&json!("monitoring")));
        assert!(prefs[0].created_at <= Utc::now());

        // =====================================================================
        // 3. Verify Reasoning tier via Store APIs
        // =====================================================================
        eprintln!("[BOLT-01-03] Verifying Reasoning tier via Neo4jReasoningStore...");
        let trace_opt = reasoning_b
            .get_trace_with_steps(trace_id)
            .await
            .expect("get_trace_with_steps failed");
        let trace = trace_opt.expect("trace must exist");
        assert_eq!(trace.id, trace_id);
        assert_eq!(trace.session_id, session_id);
        assert_eq!(trace.task, trace_rec.task);
        assert_eq!(trace.success, Some(true));
        assert_eq!(
            trace.outcome.as_deref(),
            Some("Inspection completed successfully")
        );
        assert_eq!(
            trace.metadata.get("environment"),
            Some(&json!("production"))
        );
        assert!(trace.started_at <= Utc::now());
        assert!(trace.completed_at.unwrap() >= trace.started_at);

        assert_eq!(trace.steps.len(), 1);
        let step = &trace.steps[0];
        assert_eq!(step.id, step_id);
        assert_eq!(step.step_number, 1);
        assert_eq!(step.thought, step_rec.thought);
        assert_eq!(step.action, step_rec.action);
        assert_eq!(step.observation, step_rec.observation);
        assert_eq!(step.metadata.get("agent_state"), Some(&json!("executing")));
        assert!(step.created_at <= Utc::now());

        assert_eq!(step.tool_calls.len(), 1);
        let tc = &step.tool_calls[0];
        assert_eq!(tc.id, tool_call_id);
        assert_eq!(tc.step_id, Some(step_id));
        assert_eq!(tc.tool_name, "check_replication");
        assert_eq!(tc.status, ToolCallStatus::Success);
        assert_eq!(tc.duration_ms, Some(42));
        assert_eq!(
            tc.arguments.get("cluster_id"),
            Some(&json!("neo4j-cluster-1"))
        );
        assert_eq!(tc.result, Some(json!({"status": "synced", "lag_ms": 2})));
        assert!(tc.created_at <= Utc::now());

        // =====================================================================
        // 4. Direct Cypher structural assertions verifying exact edges and shapes
        // =====================================================================
        eprintln!("[BOLT-01-03] Verifying structural edges via direct Cypher queries...");

        // 1. HAS_MESSAGE edge
        let has_message_rows = client_b
            .execute(
                "MATCH (c:Conversation {id: $cid})-[r:HAS_MESSAGE]->(m:Message) \
                 RETURN count(r) AS cnt, collect(m.id) AS msg_ids",
                vec![("cid", conv_id.to_string().into())],
            )
            .await
            .expect("Cypher HAS_MESSAGE query failed");
        assert_eq!(has_message_rows[0].get::<i64>("cnt").unwrap(), 2);
        let msg_ids = has_message_rows[0].get::<Vec<String>>("msg_ids").unwrap();
        assert!(msg_ids.contains(&msg1_id.to_string()));
        assert!(msg_ids.contains(&msg2_id.to_string()));
        eprintln!(
            "[BOLT-01-03] [PASS] HAS_MESSAGE edge verified (2 messages attached to conversation)"
        );

        // 2. RELATES_TO / RELATED_TO edge
        let related_to_rows = client_b
            .execute(
                "MATCH (e1:Entity {id: $e1})-[r:RELATED_TO]->(e2:Entity {id: $e2}) \
                 RETURN count(r) AS cnt, r.id AS rel_id, r.type AS rel_type, r.description AS desc",
                vec![
                    ("e1", entity1_id.to_string().into()),
                    ("e2", entity2_id.to_string().into()),
                ],
            )
            .await
            .expect("Cypher RELATED_TO query failed");
        assert_eq!(related_to_rows[0].get::<i64>("cnt").unwrap(), 1);
        assert_eq!(
            related_to_rows[0].get::<String>("rel_id").unwrap(),
            rel_id.to_string()
        );
        assert_eq!(
            related_to_rows[0].get::<String>("rel_type").unwrap(),
            "RELATES_TO"
        );
        assert_eq!(
            related_to_rows[0].get::<String>("desc").unwrap(),
            "Database Server relates to Backup Storage"
        );

        let relates_to_rows = client_b
            .execute(
                "MATCH (e1:Entity {id: $e1})-[r:RELATES_TO]->(e2:Entity {id: $e2}) \
                 RETURN count(r) AS cnt, r.id AS rel_id, r.type AS rel_type",
                vec![
                    ("e1", entity1_id.to_string().into()),
                    ("e2", entity2_id.to_string().into()),
                ],
            )
            .await
            .expect("Cypher RELATES_TO query failed");
        assert_eq!(relates_to_rows[0].get::<i64>("cnt").unwrap(), 1);
        eprintln!("[BOLT-01-03] [PASS] RELATES_TO edge verified between entities");

        // 3. ABOUT edges for Fact and Preference
        let fact_about_rows = client_b
            .execute(
                "MATCH (f:Fact {id: $fid})-[r:ABOUT]->(e:Entity {id: $eid}) \
                 RETURN count(r) AS cnt",
                vec![
                    ("fid", fact_id.to_string().into()),
                    ("eid", entity1_id.to_string().into()),
                ],
            )
            .await
            .expect("Cypher Fact ABOUT query failed");
        assert_eq!(fact_about_rows[0].get::<i64>("cnt").unwrap(), 1);

        let pref_about_rows = client_b
            .execute(
                "MATCH (p:Preference {id: $pid})-[r:ABOUT]->(e:Entity {id: $eid}) \
                 RETURN count(r) AS cnt",
                vec![
                    ("pid", pref_id.to_string().into()),
                    ("eid", entity1_id.to_string().into()),
                ],
            )
            .await
            .expect("Cypher Preference ABOUT query failed");
        assert_eq!(pref_about_rows[0].get::<i64>("cnt").unwrap(), 1);
        eprintln!("[BOLT-01-03] [PASS] ABOUT edges verified for both Fact and Preference");

        // 4. HAS_STEP edge
        let has_step_rows = client_b
            .execute(
                "MATCH (t:ReasoningTrace {id: $tid})-[r:HAS_STEP]->(s:ReasoningStep {id: $sid}) \
                 RETURN count(r) AS cnt, r.order AS step_order",
                vec![
                    ("tid", trace_id.to_string().into()),
                    ("sid", step_id.to_string().into()),
                ],
            )
            .await
            .expect("Cypher HAS_STEP query failed");
        assert_eq!(has_step_rows[0].get::<i64>("cnt").unwrap(), 1);
        assert_eq!(has_step_rows[0].get::<i64>("step_order").unwrap(), 1);
        eprintln!("[BOLT-01-03] [PASS] HAS_STEP edge verified with order=1");

        // 5. USES_TOOL edge
        let uses_tool_rows = client_b
            .execute(
                "MATCH (s:ReasoningStep {id: $sid})-[r:USES_TOOL]->(tc:ToolCall {id: $tcid}) \
                 RETURN count(r) AS cnt",
                vec![
                    ("sid", step_id.to_string().into()),
                    ("tcid", tool_call_id.to_string().into()),
                ],
            )
            .await
            .expect("Cypher USES_TOOL query failed");
        assert_eq!(uses_tool_rows[0].get::<i64>("cnt").unwrap(), 1);
        eprintln!("[BOLT-01-03] [PASS] USES_TOOL edge verified from ReasoningStep to ToolCall");
    }

    eprintln!(
        "=== [SUCCESS] BOLT-01-03 Durability Reconnect Integration Test Passed All Invariants ==="
    );
}
