//! Integration test for BOLT-03-03: Multi-tenant boundary and strict owner
//! scope isolation across ShortTerm, LongTerm, and Reasoning tiers.
//!
//! Operates against an isolated, authenticated, loopback Neo4j 5.26 container,
//! torn down cleanly upon completion via a drop guard. Credentials are kept
//! private and never logged.

use chrono::Utc;
use muninn_core::model::{Entity, Fact, Metadata, Preference, ToolCall, ToolCallStatus};
use muninn_core::ports::{
    EmbeddingIdentity, LongTermStore, MessageRecord, OwnerScope, ReasoningStore, SessionQuery,
    ShortTermStore, StepRecord, TraceQuery, TraceRecord,
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
        if let Ok(uri) = std::env::var("MUNINN_FIXTURE_URI") {
            let password = std::env::var("MUNINN_FIXTURE_PASSWORD").unwrap_or_default();
            eprintln!("[BOLT-03-03] Using existing Neo4j fixture at {}", uri);
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

        eprintln!("[BOLT-03-03] Spawning isolated neo4j:5.26-community container via loopback...");
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
            "[BOLT-03-03] Container {} running at 127.0.0.1:{}",
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
            eprintln!("[BOLT-03-03] Cleaning up container {}...", container_id);
            let _ = Command::new("docker")
                .args(["stop", "-t", "2", container_id])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            eprintln!("[BOLT-03-03] Container cleanup complete.");
        }
    }
}

async fn wait_for_neo4j(config: &Neo4jConfig) -> Neo4jClient {
    for attempt in 1..=60 {
        if let Ok(client) = Neo4jClient::connect(config).await {
            if client.execute("RETURN 1 AS n", vec![]).await.is_ok() {
                eprintln!("[BOLT-03-03] Neo4j ready after attempt {}", attempt);
                return client;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("Neo4j did not become ready in 30s");
}

#[tokio::test]
async fn test_bolt_03_03_multi_tenant_isolation_across_three_tiers() {
    eprintln!("=== [START] BOLT-03-03 Multi-Tenant Strict Isolation Test ===");
    let fixture = DockerFixture::start();
    let config = Neo4jConfig {
        uri: fixture.uri.clone(),
        username: "neo4j".into(),
        password: fixture.password.clone(),
        ..Default::default()
    };

    let client = wait_for_neo4j(&config).await;

    // Apply SchemaManager::setup_all with vector dimension 3
    eprintln!("[BOLT-03-03] Applying SchemaManager::setup_all with 3 dimensions...");
    let schema_mgr = SchemaManager::new(&client, 3);
    schema_mgr
        .setup_all()
        .await
        .expect("SchemaManager setup_all failed");

    client
        .execute("CALL db.awaitIndexes(60)", vec![])
        .await
        .expect("awaitIndexes failed");

    let short_term = Neo4jShortTermStore::new(client.clone());
    let long_term = Neo4jLongTermStore::new(client.clone());
    let reasoning = Neo4jReasoningStore::new(client.clone());

    // Define two distinct tenants / owners
    let tenant_alpha_str = "tenant-alpha";
    let tenant_beta_str = "tenant-beta";
    let user_alpha_str = "user-alpha";
    let user_beta_str = "user-beta";

    let scope_alpha = OwnerScope::owned(tenant_alpha_str).expect("valid scope alpha");
    let scope_beta = OwnerScope::owned(tenant_beta_str).expect("valid scope beta");

    let session_alpha = format!("session-alpha-{}", Uuid::new_v4());
    let session_beta = format!("session-beta-{}", Uuid::new_v4());

    let embedding_model = "test:embedding-model:v1";
    let embedding_identity = EmbeddingIdentity::new(embedding_model, 3).expect("valid identity");
    // Shared identical embedding vector: tests that vector search NEVER leaks across tenants
    let shared_vector = vec![1.0, 0.0, 0.0];

    // =========================================================================
    // 1. Populate full data for tenant-alpha
    // =========================================================================
    eprintln!("[BOLT-03-03] --- Populating complete data for tenant-alpha ---");

    // ShortTerm: Conversation + 2 Messages
    let conv_alpha_id = Uuid::new_v4();
    let created_conv_alpha = short_term
        .ensure_conversation(&session_alpha, Some(conv_alpha_id), &scope_alpha)
        .await
        .expect("ensure conv alpha failed");
    assert_eq!(created_conv_alpha, conv_alpha_id);

    let msg_alpha1_id = Uuid::new_v4();
    let mut msg_alpha1_meta = Metadata::new();
    msg_alpha1_meta.insert("tenant".into(), json!("alpha"));
    msg_alpha1_meta.insert("security".into(), json!("restricted"));
    let msg_alpha1 = MessageRecord {
        operation_key: Some("alpha-msg-1".into()),
        operation_payload: Some("payload-alpha-1".into()),
        id: msg_alpha1_id,
        role: "user".into(),
        content: "Alpha secret project requirements".into(),
        embedding: Some(shared_vector.clone()),
        embedding_identity: Some(embedding_identity.clone()),
        metadata: msg_alpha1_meta.clone(),
        timestamp: Some(Utc::now()),
    };
    short_term
        .append_message(conv_alpha_id, &scope_alpha, &msg_alpha1)
        .await
        .expect("append msg_alpha1 failed");

    let msg_alpha2_id = Uuid::new_v4();
    let mut msg_alpha2_meta = Metadata::new();
    msg_alpha2_meta.insert("tenant".into(), json!("alpha"));
    let msg_alpha2 = MessageRecord {
        operation_key: Some("alpha-msg-2".into()),
        operation_payload: Some("payload-alpha-2".into()),
        id: msg_alpha2_id,
        role: "assistant".into(),
        content: "Alpha project requirements acknowledged and encrypted".into(),
        embedding: Some(shared_vector.clone()),
        embedding_identity: Some(embedding_identity.clone()),
        metadata: msg_alpha2_meta.clone(),
        timestamp: Some(Utc::now()),
    };
    short_term
        .append_message(conv_alpha_id, &scope_alpha, &msg_alpha2)
        .await
        .expect("append msg_alpha2 failed");

    // LongTerm: Entity + Fact + Preference linked to user-alpha
    let entity_alpha_id = Uuid::new_v4();
    let entity_alpha = Entity {
        id: entity_alpha_id,
        name: "Alpha Secret Infrastructure".into(),
        canonical_name: Some("alpha-infra-01".into()),
        entity_type: "INFRASTRUCTURE".into(),
        subtype: Some("neo4j-alpha".into()),
        description: Some("Alpha confidential database".into()),
        confidence: 0.99,
        aliases: vec!["alpha-db".into()],
        embedding: None,
        created_at: Utc::now(),
        updated_at: None,
        metadata: Metadata::new(),
    };
    long_term
        .upsert_entity(&entity_alpha)
        .await
        .expect("upsert entity_alpha failed");

    let fact_alpha_id = Uuid::new_v4();
    let fact_alpha = Fact {
        id: fact_alpha_id,
        subject: "Alpha Secret Infrastructure".into(),
        predicate: "ACCESS_LEVEL".into(),
        object: "ALPHA_PERSONNEL_ONLY".into(),
        confidence: 1.0,
        embedding: None,
        valid_from: None,
        valid_until: None,
        created_at: Utc::now(),
        metadata: Metadata::new(),
    };
    long_term
        .create_fact_with_about(&fact_alpha, &[entity_alpha_id])
        .await
        .expect("create fact_alpha failed");

    let pref_alpha_id = Uuid::new_v4();
    let pref_alpha = Preference {
        id: pref_alpha_id,
        category: "authentication".into(),
        preference: "Alpha requires hardware 2FA tokens".into(),
        context: Some("identity verification".into()),
        confidence: 0.98,
        embedding: None,
        valid_from: None,
        valid_until: None,
        created_at: Utc::now(),
        metadata: Metadata::new(),
    };
    long_term
        .create_preference(&pref_alpha)
        .await
        .expect("create pref_alpha failed");
    long_term
        .link_user_to_preference(user_alpha_str, pref_alpha_id)
        .await
        .expect("link pref_alpha to user_alpha failed");
    long_term
        .link_preference_about_entity(pref_alpha_id, entity_alpha_id)
        .await
        .expect("link pref_alpha about entity_alpha failed");
    long_term
        .link_preference_to_entity(pref_alpha_id, entity_alpha_id)
        .await
        .expect("link pref_alpha applies_to entity_alpha failed");

    // Reasoning: ReasoningTrace + Step + ToolCall
    let trace_alpha_id = Uuid::new_v4();
    let trace_alpha_rec = TraceRecord {
        id: trace_alpha_id,
        session_id: session_alpha.clone(),
        task: "Execute confidential alpha deployment".into(),
        task_embedding: None,
        metadata: Metadata::new(),
    };
    let created_trace_alpha = reasoning
        .create_trace(&trace_alpha_rec, &scope_alpha, Some(msg_alpha1_id))
        .await
        .expect("create trace_alpha failed");
    assert_eq!(created_trace_alpha.id, trace_alpha_id);

    reasoning
        .link_conversation_to_trace(conv_alpha_id, trace_alpha_id, &scope_alpha)
        .await
        .expect("link conv_alpha to trace_alpha failed");

    let step_alpha_id = Uuid::new_v4();
    let step_alpha_rec = StepRecord {
        id: step_alpha_id,
        thought: Some("Verifying alpha infrastructure security".into()),
        action: Some("Invoke alpha_deploy_tool".into()),
        observation: Some("Alpha infrastructure secure".into()),
        embedding: None,
        metadata: Metadata::new(),
    };
    reasoning
        .add_step(trace_alpha_id, &step_alpha_rec)
        .await
        .expect("add step_alpha failed");

    let tool_call_alpha_id = Uuid::new_v4();
    let tool_call_alpha = ToolCall {
        id: tool_call_alpha_id,
        step_id: Some(step_alpha_id),
        tool_name: "alpha_deploy_tool".into(),
        arguments: Metadata::new(),
        result: Some(json!({"status": "alpha_ready"})),
        status: ToolCallStatus::Success,
        duration_ms: Some(15),
        error: None,
        created_at: Utc::now(),
    };
    reasoning
        .record_tool_call(&tool_call_alpha, &scope_alpha, Some(msg_alpha1_id))
        .await
        .expect("record tool_call_alpha failed");

    reasoning
        .complete_trace(
            trace_alpha_id,
            Some("Alpha deployment successful"),
            Some(true),
        )
        .await
        .expect("complete trace_alpha failed");

    // =========================================================================
    // 2. Populate full data for tenant-beta
    // =========================================================================
    eprintln!("[BOLT-03-03] --- Populating complete data for tenant-beta ---");

    // ShortTerm: Conversation + 2 Messages
    let conv_beta_id = Uuid::new_v4();
    let created_conv_beta = short_term
        .ensure_conversation(&session_beta, Some(conv_beta_id), &scope_beta)
        .await
        .expect("ensure conv beta failed");
    assert_eq!(created_conv_beta, conv_beta_id);

    let msg_beta1_id = Uuid::new_v4();
    let mut msg_beta1_meta = Metadata::new();
    msg_beta1_meta.insert("tenant".into(), json!("beta"));
    msg_beta1_meta.insert("security".into(), json!("confidential"));
    let msg_beta1 = MessageRecord {
        operation_key: Some("beta-msg-1".into()),
        operation_payload: Some("payload-beta-1".into()),
        id: msg_beta1_id,
        role: "user".into(),
        content: "Beta proprietary intellectual property and patents".into(),
        // EXACT same vector and identity as alpha: proves isolation is strictly enforced
        embedding: Some(shared_vector.clone()),
        embedding_identity: Some(embedding_identity.clone()),
        metadata: msg_beta1_meta.clone(),
        timestamp: Some(Utc::now()),
    };
    short_term
        .append_message(conv_beta_id, &scope_beta, &msg_beta1)
        .await
        .expect("append msg_beta1 failed");

    let msg_beta2_id = Uuid::new_v4();
    let mut msg_beta2_meta = Metadata::new();
    msg_beta2_meta.insert("tenant".into(), json!("beta"));
    let msg_beta2 = MessageRecord {
        operation_key: Some("beta-msg-2".into()),
        operation_payload: Some("payload-beta-2".into()),
        id: msg_beta2_id,
        role: "assistant".into(),
        content: "Beta IP securely filed and indexed".into(),
        embedding: Some(shared_vector.clone()),
        embedding_identity: Some(embedding_identity.clone()),
        metadata: msg_beta2_meta.clone(),
        timestamp: Some(Utc::now()),
    };
    short_term
        .append_message(conv_beta_id, &scope_beta, &msg_beta2)
        .await
        .expect("append msg_beta2 failed");

    // LongTerm: Entity + Fact + Preference linked to user-beta
    let entity_beta_id = Uuid::new_v4();
    let entity_beta = Entity {
        id: entity_beta_id,
        name: "Beta Proprietary Storage".into(),
        canonical_name: Some("beta-storage-01".into()),
        entity_type: "STORAGE".into(),
        subtype: Some("secure-vault".into()),
        description: Some("Beta proprietary patent storage".into()),
        confidence: 0.97,
        aliases: vec!["beta-vault".into()],
        embedding: None,
        created_at: Utc::now(),
        updated_at: None,
        metadata: Metadata::new(),
    };
    long_term
        .upsert_entity(&entity_beta)
        .await
        .expect("upsert entity_beta failed");

    let fact_beta_id = Uuid::new_v4();
    let fact_beta = Fact {
        id: fact_beta_id,
        subject: "Beta Proprietary Storage".into(),
        predicate: "CLASSIFICATION".into(),
        object: "BETA_EXCLUSIVE".into(),
        confidence: 1.0,
        embedding: None,
        valid_from: None,
        valid_until: None,
        created_at: Utc::now(),
        metadata: Metadata::new(),
    };
    long_term
        .create_fact_with_about(&fact_beta, &[entity_beta_id])
        .await
        .expect("create fact_beta failed");

    let pref_beta_id = Uuid::new_v4();
    let pref_beta = Preference {
        id: pref_beta_id,
        category: "authentication".into(),
        preference: "Beta requires biometric facial recognition".into(),
        context: Some("physical access".into()),
        confidence: 0.96,
        embedding: None,
        valid_from: None,
        valid_until: None,
        created_at: Utc::now(),
        metadata: Metadata::new(),
    };
    long_term
        .create_preference(&pref_beta)
        .await
        .expect("create pref_beta failed");
    long_term
        .link_user_to_preference(user_beta_str, pref_beta_id)
        .await
        .expect("link pref_beta to user_beta failed");
    long_term
        .link_preference_about_entity(pref_beta_id, entity_beta_id)
        .await
        .expect("link pref_beta about entity_beta failed");
    long_term
        .link_preference_to_entity(pref_beta_id, entity_beta_id)
        .await
        .expect("link pref_beta applies_to entity_beta failed");

    // Reasoning: ReasoningTrace + Step + ToolCall
    let trace_beta_id = Uuid::new_v4();
    let trace_beta_rec = TraceRecord {
        id: trace_beta_id,
        session_id: session_beta.clone(),
        task: "Execute proprietary beta compilation".into(),
        task_embedding: None,
        metadata: Metadata::new(),
    };
    let created_trace_beta = reasoning
        .create_trace(&trace_beta_rec, &scope_beta, Some(msg_beta1_id))
        .await
        .expect("create trace_beta failed");
    assert_eq!(created_trace_beta.id, trace_beta_id);

    reasoning
        .link_conversation_to_trace(conv_beta_id, trace_beta_id, &scope_beta)
        .await
        .expect("link conv_beta to trace_beta failed");

    let step_beta_id = Uuid::new_v4();
    let step_beta_rec = StepRecord {
        id: step_beta_id,
        thought: Some("Compiling beta proprietary algorithms".into()),
        action: Some("Invoke beta_compile_tool".into()),
        observation: Some("Beta binaries built successfully".into()),
        embedding: None,
        metadata: Metadata::new(),
    };
    reasoning
        .add_step(trace_beta_id, &step_beta_rec)
        .await
        .expect("add step_beta failed");

    let tool_call_beta_id = Uuid::new_v4();
    let tool_call_beta = ToolCall {
        id: tool_call_beta_id,
        step_id: Some(step_beta_id),
        tool_name: "beta_compile_tool".into(),
        arguments: Metadata::new(),
        result: Some(json!({"artifact_hash": "sha256:beta-confidential"})),
        status: ToolCallStatus::Success,
        duration_ms: Some(88),
        error: None,
        created_at: Utc::now(),
    };
    reasoning
        .record_tool_call(&tool_call_beta, &scope_beta, Some(msg_beta1_id))
        .await
        .expect("record tool_call_beta failed");

    // =========================================================================
    // 3. STRICT READ ISOLATION VALIDATIONS (Querying as tenant-alpha)
    // =========================================================================
    eprintln!("[BOLT-03-03] --- Validating Strict Read Isolation from tenant-alpha ---");

    // 3.1 ShortTerm Read Isolation
    eprintln!("[BOLT-03-03] Validating ShortTerm read isolation...");
    // Direct get by conversation ID of foreign tenant
    let conv_cross_id = short_term
        .get_conversation_by_id(conv_beta_id, &scope_alpha)
        .await
        .expect("get_conversation_by_id query must succeed");
    assert!(
        conv_cross_id.is_none(),
        "tenant-alpha MUST NOT be able to read tenant-beta's conversation by ID"
    );

    // Get by session ID of foreign tenant
    let conv_cross_session = short_term
        .get_conversation(&session_beta, None, &scope_alpha, None)
        .await
        .expect("get_conversation query must succeed");
    assert!(
        conv_cross_session.is_none(),
        "tenant-alpha MUST NOT be able to read tenant-beta's conversation by session"
    );

    // Get by foreign conversation ID and session
    let conv_cross_both = short_term
        .get_conversation(&session_beta, Some(conv_beta_id), &scope_alpha, None)
        .await
        .expect("get_conversation query must succeed");
    assert!(
        conv_cross_both.is_none(),
        "tenant-alpha MUST NOT be able to read tenant-beta's conversation by session+id"
    );

    // list_conversations for tenant-alpha
    let alpha_convs = short_term
        .list_conversations(&scope_alpha, 100)
        .await
        .expect("list_conversations for alpha failed");
    assert_eq!(
        alpha_convs.len(),
        1,
        "list_conversations for alpha must return exactly 1 conversation"
    );
    assert_eq!(alpha_convs[0].id, conv_alpha_id);
    assert!(
        alpha_convs.iter().all(|c| c.id != conv_beta_id),
        "list_conversations for alpha MUST NOT contain beta's conversation"
    );

    // list_sessions for tenant-alpha
    let alpha_sessions = short_term
        .list_sessions(
            &SessionQuery {
                limit: 10,
                ..SessionQuery::default()
            },
            &scope_alpha,
        )
        .await
        .expect("list_sessions for alpha failed");
    assert_eq!(alpha_sessions.len(), 1);
    assert_eq!(alpha_sessions[0].session_id, session_alpha);
    assert!(
        alpha_sessions.iter().all(|s| s.session_id != session_beta),
        "list_sessions for alpha MUST NOT include beta's session"
    );

    // search_messages with identical vector [1.0, 0.0, 0.0] as tenant-alpha:
    // Beta's message has the EXACT same embedding, but alpha's search MUST NOT return beta's messages!
    let alpha_hits = short_term
        .search_messages_with_identity(
            &shared_vector,
            &embedding_identity,
            None,
            &scope_alpha,
            10,
            0.0,
            &Metadata::new(),
        )
        .await
        .expect("search_messages_with_identity failed");
    assert_eq!(
        alpha_hits.len(),
        2,
        "search_messages for alpha must return only alpha's 2 messages"
    );
    for hit in &alpha_hits {
        assert!(
            hit.message.id == msg_alpha1_id || hit.message.id == msg_alpha2_id,
            "Found leaked foreign message: {:?}",
            hit.message.id
        );
        assert_ne!(
            hit.message.id, msg_beta1_id,
            "CRITICAL: tenant-beta msg1 leaked to tenant-alpha via vector search!"
        );
        assert_ne!(
            hit.message.id, msg_beta2_id,
            "CRITICAL: tenant-beta msg2 leaked to tenant-alpha via vector search!"
        );
    }

    // Unidentified search_messages must fail closed with error
    let unident_search = short_term
        .search_messages(
            &shared_vector,
            None,
            &scope_alpha,
            10,
            0.0,
            &Metadata::new(),
        )
        .await;
    assert!(
        unident_search.is_err(),
        "unidentified search_messages must be rejected"
    );

    // 3.2 LongTerm Read Isolation
    eprintln!("[BOLT-03-03] Validating LongTerm preference read isolation...");
    let prefs_alpha = long_term
        .preferences_for(user_alpha_str, false)
        .await
        .expect("preferences_for user-alpha failed");
    assert_eq!(prefs_alpha.len(), 1);
    assert_eq!(prefs_alpha[0].id, pref_alpha_id);
    assert_ne!(prefs_alpha[0].id, pref_beta_id);

    // Querying preferences for user-alpha filtered by beta's entity MUST return empty
    let prefs_cross_target = long_term
        .preferences_for_filtered(user_alpha_str, false, Some(entity_beta_id), None)
        .await
        .expect("preferences_for_filtered failed");
    assert!(
        prefs_cross_target.is_empty(),
        "user-alpha has no preferences applied to beta's entity"
    );

    // Category query scoped to user-alpha
    let prefs_alpha_cat = long_term
        .preferences_by_category("authentication", 10, Some(user_alpha_str))
        .await
        .expect("preferences_by_category failed");
    assert_eq!(prefs_alpha_cat.len(), 1);
    assert_eq!(prefs_alpha_cat[0].id, pref_alpha_id);
    assert_ne!(prefs_alpha_cat[0].id, pref_beta_id);

    // 3.3 Reasoning Read Isolation
    eprintln!("[BOLT-03-03] Validating Reasoning read isolation...");
    // open_conversation_trace for beta's conversation requested by alpha
    let cross_open_trace = reasoning
        .open_conversation_trace(conv_beta_id, &scope_alpha)
        .await
        .expect("open_conversation_trace failed");
    assert!(
        cross_open_trace.is_none(),
        "tenant-alpha MUST NOT see open traces of tenant-beta's conversation"
    );

    // conversation_steps for beta's conversation requested by alpha
    let cross_steps = reasoning
        .conversation_steps(conv_beta_id, &scope_alpha)
        .await
        .expect("conversation_steps query failed");
    assert!(
        cross_steps.is_none(),
        "tenant-alpha MUST NOT see conversation steps of tenant-beta's conversation"
    );

    // step_context for beta's step requested by alpha
    let cross_step_ctx = reasoning
        .step_context(step_beta_id, &scope_alpha)
        .await
        .expect("step_context query failed");
    assert!(
        cross_step_ctx.is_none(),
        "tenant-alpha MUST NOT see step context for beta's step"
    );

    // validate_message_scope on beta's message with alpha scope must error
    assert!(
        reasoning
            .validate_message_scope(msg_beta1_id, &scope_alpha)
            .await
            .is_err(),
        "validate_message_scope on foreign message MUST fail"
    );

    // list_traces filtered by session_alpha
    let alpha_traces = reasoning
        .list_traces(&TraceQuery {
            session_id: Some(session_alpha.clone()),
            ..Default::default()
        })
        .await
        .expect("list_traces for alpha failed");
    assert_eq!(alpha_traces.len(), 1);
    assert_eq!(alpha_traces[0].id, trace_alpha_id);

    // =========================================================================
    // 4. MALICIOUS MUTATION AND DELETION ATTEMPTS BY tenant-alpha AGAINST tenant-beta
    // =========================================================================
    eprintln!(
        "[BOLT-03-03] --- Executing Malicious Mutation/Deletion Attempts by tenant-alpha ---"
    );

    // 4.1 Attempt to adopt tenant-beta's conversation under tenant-alpha scope
    let adopt_attempt = short_term
        .ensure_conversation(&session_alpha, Some(conv_beta_id), &scope_alpha)
        .await;
    assert!(
        adopt_attempt.is_err(),
        "tenant-alpha MUST NOT be able to adopt tenant-beta's conversation"
    );

    // 4.2 Attempt to append message to tenant-beta's conversation as tenant-alpha
    let malicious_msg_id = Uuid::new_v4();
    let malicious_msg = MessageRecord {
        operation_key: Some("malicious-1".into()),
        operation_payload: Some("malicious-payload".into()),
        id: malicious_msg_id,
        role: "user".into(),
        content: "Malicious injection into beta conversation".into(),
        embedding: None,
        embedding_identity: None,
        metadata: Metadata::new(),
        timestamp: Some(Utc::now()),
    };
    let append_single_attempt = short_term
        .append_message(conv_beta_id, &scope_alpha, &malicious_msg)
        .await;
    assert!(
        append_single_attempt.is_err(),
        "tenant-alpha MUST NOT append message to tenant-beta's conversation"
    );

    let append_batch_attempt = short_term
        .append_messages(conv_beta_id, &scope_alpha, &[malicious_msg])
        .await;
    assert!(
        append_batch_attempt.is_err(),
        "tenant-alpha MUST NOT batch append to tenant-beta's conversation"
    );

    // 4.3 Attempt to delete tenant-beta's message as tenant-alpha
    let delete_msg_attempt = short_term
        .delete_message(msg_beta1_id, &scope_alpha, true)
        .await
        .expect("delete_message call executes");
    assert!(
        !delete_msg_attempt,
        "delete_message by tenant-alpha against beta's message must return false"
    );

    // 4.4 Attempt to delete tenant-beta's conversation as tenant-alpha
    // delete_conversation matches on owner_scope; it must safely be a no-op
    short_term
        .delete_conversation(conv_beta_id, &scope_alpha)
        .await
        .expect("delete_conversation by alpha should complete safely without deleting");

    // 4.5 Attempt to clear tenant-beta's session as tenant-alpha
    short_term
        .clear_session(&session_beta, &scope_alpha)
        .await
        .expect("clear_session by alpha should complete safely without deleting");

    // 4.6 Attempt to link or mutate beta's reasoning tier
    let malicious_trace_rec = TraceRecord {
        id: Uuid::new_v4(),
        session_id: session_alpha.clone(),
        task: "Malicious trace cross-link".into(),
        task_embedding: None,
        metadata: Metadata::new(),
    };
    // Cannot initiate a trace from beta's message
    let cross_trace_attempt = reasoning
        .create_trace(&malicious_trace_rec, &scope_alpha, Some(msg_beta1_id))
        .await;
    assert!(
        cross_trace_attempt.is_err(),
        "tenant-alpha MUST NOT initiate a trace from tenant-beta's message"
    );

    // Cannot add conversation step to beta's conversation
    let malicious_step_rec = StepRecord {
        id: Uuid::new_v4(),
        thought: Some("Malicious step".into()),
        action: None,
        observation: None,
        embedding: None,
        metadata: Metadata::new(),
    };
    let add_conv_step_attempt = reasoning
        .add_conversation_step(
            conv_beta_id,
            trace_alpha_id,
            &scope_alpha,
            &malicious_step_rec,
        )
        .await;
    assert!(
        add_conv_step_attempt.is_err(),
        "tenant-alpha MUST NOT add step to tenant-beta's conversation"
    );

    // ensure_conversation_trace on beta's conversation with alpha scope must error
    let ensure_trace_attempt = reasoning
        .ensure_conversation_trace(conv_beta_id, &malicious_trace_rec, &scope_alpha)
        .await;
    assert!(
        ensure_trace_attempt.is_err(),
        "tenant-alpha MUST NOT ensure trace on tenant-beta's conversation"
    );

    // Linking beta's conversation to alpha's trace must return false
    let link_conv_attempt = reasoning
        .link_conversation_to_trace(conv_beta_id, trace_alpha_id, &scope_alpha)
        .await
        .expect("link call succeeds");
    assert!(
        !link_conv_attempt,
        "link_conversation_to_trace across tenants must return false"
    );

    // Linking beta's trace to alpha's message must return false
    let link_trace_attempt = reasoning
        .link_trace_to_message(trace_beta_id, msg_alpha1_id, &scope_alpha)
        .await
        .expect("link call succeeds");
    assert!(
        !link_trace_attempt,
        "link_trace_to_message across tenants must return false"
    );

    // Linking beta's tool call to alpha's message must return false
    let link_tool_attempt = reasoning
        .link_tool_call_to_message(tool_call_beta_id, msg_alpha1_id, &scope_alpha)
        .await
        .expect("link call succeeds");
    assert!(
        !link_tool_attempt,
        "link_tool_call_to_message across tenants must return false"
    );

    // =========================================================================
    // 5. POST-ATTACK INTEGRITY VERIFICATION OF tenant-beta's DATA
    // =========================================================================
    eprintln!(
        "[BOLT-03-03] --- Verifying 100% Integrity of tenant-beta Data After Attack Attempts ---"
    );

    // 5.1 ShortTerm: tenant-beta conversation and both messages remain 100% intact
    let conv_beta_by_id = short_term
        .get_conversation_by_id(conv_beta_id, &scope_beta)
        .await
        .expect("get_conversation_by_id for beta failed")
        .expect("tenant-beta conversation must still exist");
    assert_eq!(conv_beta_by_id.id, conv_beta_id);
    assert_eq!(conv_beta_by_id.session_id, session_beta);

    let conv_beta_verified = short_term
        .get_conversation(&session_beta, Some(conv_beta_id), &scope_beta, Some(10))
        .await
        .expect("get_conversation for beta failed")
        .expect("tenant-beta conversation must still exist with messages");
    assert_eq!(conv_beta_verified.id, conv_beta_id);
    assert_eq!(conv_beta_verified.session_id, session_beta);
    assert_eq!(
        conv_beta_verified.messages.len(),
        2,
        "tenant-beta must still have exactly 2 messages"
    );
    assert_eq!(conv_beta_verified.messages[0].id, msg_beta1_id);
    assert_eq!(
        conv_beta_verified.messages[0].content,
        "Beta proprietary intellectual property and patents"
    );
    assert_eq!(conv_beta_verified.messages[1].id, msg_beta2_id);
    assert_eq!(
        conv_beta_verified.messages[1].content,
        "Beta IP securely filed and indexed"
    );

    // 5.2 LongTerm: Entity, Fact, and Preference remain 100% intact
    let entity_beta_verified = long_term
        .get_entity(entity_beta_id)
        .await
        .expect("get_entity beta failed")
        .expect("entity_beta must exist");
    assert_eq!(entity_beta_verified.id, entity_beta_id);
    assert_eq!(entity_beta_verified.name, "Beta Proprietary Storage");

    let facts_beta_verified = long_term
        .facts_by_subject("Beta Proprietary Storage", 10)
        .await
        .expect("facts_by_subject beta failed");
    assert_eq!(facts_beta_verified.len(), 1);
    assert_eq!(facts_beta_verified[0].id, fact_beta_id);
    assert_eq!(facts_beta_verified[0].object, "BETA_EXCLUSIVE");

    let prefs_beta_verified = long_term
        .preferences_for(user_beta_str, false)
        .await
        .expect("preferences_for user-beta failed");
    assert_eq!(prefs_beta_verified.len(), 1);
    assert_eq!(prefs_beta_verified[0].id, pref_beta_id);
    assert_eq!(
        prefs_beta_verified[0].preference,
        "Beta requires biometric facial recognition"
    );

    let prefs_beta_filtered = long_term
        .preferences_for_filtered(user_beta_str, false, Some(entity_beta_id), None)
        .await
        .expect("preferences_for_filtered beta failed");
    assert_eq!(prefs_beta_filtered.len(), 1);
    assert_eq!(prefs_beta_filtered[0].id, pref_beta_id);

    // 5.3 Reasoning: Trace, Step, ToolCall remain 100% intact
    let trace_beta_verified = reasoning
        .get_trace_with_steps(trace_beta_id)
        .await
        .expect("get_trace_with_steps beta failed")
        .expect("trace_beta must exist");
    assert_eq!(trace_beta_verified.id, trace_beta_id);
    assert_eq!(
        trace_beta_verified.task,
        "Execute proprietary beta compilation"
    );
    assert_eq!(trace_beta_verified.steps.len(), 1);
    assert_eq!(trace_beta_verified.steps[0].id, step_beta_id);

    let conv_steps_beta = reasoning
        .conversation_steps(conv_beta_id, &scope_beta)
        .await
        .expect("conversation_steps beta failed")
        .expect("conversation steps must exist");
    assert_eq!(conv_steps_beta.len(), 1);
    assert_eq!(conv_steps_beta[0].id, step_beta_id);
    assert_eq!(conv_steps_beta[0].tool_calls.len(), 1);
    assert_eq!(conv_steps_beta[0].tool_calls[0].id, tool_call_beta_id);
    assert_eq!(
        conv_steps_beta[0].tool_calls[0].tool_name,
        "beta_compile_tool"
    );

    // Complete trace_beta now and confirm completion succeeds
    let completed_beta = reasoning
        .complete_trace(trace_beta_id, Some("Beta compilation finished"), Some(true))
        .await
        .expect("complete_trace beta failed")
        .expect("trace must be returned");
    assert_eq!(completed_beta.success, Some(true));
    assert_eq!(
        completed_beta.outcome.as_deref(),
        Some("Beta compilation finished")
    );

    eprintln!(
        "[BOLT-03-03] === All Multi-Tenant Strict Isolation Assertions PASSED Successfully! ==="
    );
}
