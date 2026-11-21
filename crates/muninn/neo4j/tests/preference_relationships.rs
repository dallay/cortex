//! Integration test for BOLT-07-05 and BOLT-07-06: Preference relationships
//! (normative ABOUT and observable APPLIES_TO with filtered reading).
//!
//! Operates against an isolated, authenticated, loopback Neo4j 5.26 container,
//! torn down cleanly upon completion via a drop guard. Credentials are kept
//! private and never logged.

use chrono::Utc;
use muninn_application::{
    AddEntityOptions, AddPreferenceOptions, MemoryClient, MemoryOptions, MemoryPorts,
};
use muninn_core::model::Preference;
use muninn_core::ports::LongTermStore;
use muninn_neo4j::{
    Neo4jClient, Neo4jConfig, Neo4jLongTermStore, Neo4jReasoningStore, Neo4jShortTermStore,
    SchemaManager,
};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

struct DockerFixture {
    container_id: Option<String>,
    pub uri: String,
    pub password: String,
    vector_dimensions: usize,
}

impl DockerFixture {
    fn start() -> Self {
        if let Ok(uri) = std::env::var("MUNINN_FIXTURE_URI") {
            let password = std::env::var("MUNINN_FIXTURE_PASSWORD").unwrap_or_default();
            // CI's shared Neo4j fixture uses the production embedding dimension.
            let vector_dimensions = 1536;
            eprintln!("[BOLT-07] Using existing Neo4j fixture");
            return Self {
                container_id: None,
                uri,
                password,
                vector_dimensions,
            };
        }

        let password = format!("p_{}", Uuid::new_v4().simple());
        let env_file = std::env::temp_dir().join(format!("neo4j_auth_{}.env", Uuid::new_v4()));
        let env_body = format!(
            "NEO4J_AUTH=neo4j/{password}\n\
             NEO4J_dbms_security_auth__minimum__password__length=4\n\
             NEO4J_db_transaction_timeout=5s\n\
             NEO4J_db_lock_acquisition_timeout=5s\n\
             NEO4J_db_transaction_monitor_check_interval=1s\n"
        );
        std::fs::write(&env_file, env_body).expect("failed to write temporary auth env file");

        eprintln!("[BOLT-07] Spawning isolated neo4j:5.26-community container via loopback...");
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

        assert!(output.status.success(), "docker run failed");

        let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        assert!(
            (12..=64).contains(&container_id.len())
                && container_id.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "docker returned an invalid container identifier"
        );

        let port_output = Command::new("docker")
            .args(["port", &container_id, "7687/tcp"])
            .output()
            .expect("failed to get docker port");

        assert!(port_output.status.success(), "docker port failed");

        let port_str = String::from_utf8_lossy(&port_output.stdout);
        let port = port_str
            .trim()
            .rsplit(':')
            .next()
            .and_then(|port| port.parse::<u16>().ok())
            .filter(|port| *port != 0)
            .expect("docker returned an invalid published port");
        let uri = format!("bolt://127.0.0.1:{port}");
        eprintln!(
            "[BOLT-07] Container {} running at 127.0.0.1:{}",
            container_id, port
        );

        Self {
            container_id: Some(container_id),
            uri,
            password,
            vector_dimensions: 3,
        }
    }
}

impl Drop for DockerFixture {
    fn drop(&mut self) {
        if let Some(ref container_id) = self.container_id {
            eprintln!("[BOLT-07] Cleaning up container {}...", container_id);
            let _ = Command::new("docker")
                .args(["stop", "-t", "2", container_id])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            eprintln!("[BOLT-07] Container cleanup complete.");
        }
    }
}

async fn wait_for_neo4j(config: &Neo4jConfig) -> Neo4jClient {
    for attempt in 1..=60 {
        if let Ok(client) = Neo4jClient::connect(config).await {
            if client.execute("RETURN 1 AS n", vec![]).await.is_ok() {
                eprintln!("[BOLT-07] Neo4j ready after attempt {}", attempt);
                return client;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("Neo4j did not become ready in 30s");
}

async fn count_nodes_edges(client: &Neo4jClient) -> (i64, i64) {
    let rows = client
        .execute(
            "MATCH (n) WITH count(n) AS nodes OPTIONAL MATCH ()-[r]->() RETURN nodes, count(r) AS edges",
            vec![],
        )
        .await
        .expect("count nodes and edges query failed");
    (
        rows[0].get::<i64>("nodes").unwrap(),
        rows[0].get::<i64>("edges").unwrap(),
    )
}

#[tokio::test]
async fn test_bolt_07_preference_relationships_about_and_applies_to() {
    eprintln!("=== [START] BOLT-07 Preference Relationships (ABOUT & APPLIES_TO) Test ===");
    let fixture = DockerFixture::start();
    let config = Neo4jConfig {
        uri: fixture.uri.clone(),
        username: "neo4j".into(),
        password: fixture.password.clone(),
        ..Default::default()
    };

    let client = wait_for_neo4j(&config).await;

    // Match the existing fixture's managed vector index configuration.
    eprintln!("[BOLT-07] Applying SchemaManager::setup_all...");
    let schema_mgr = SchemaManager::new(&client, fixture.vector_dimensions);
    schema_mgr
        .setup_all()
        .await
        .expect("SchemaManager setup_all failed");

    client
        .execute("CALL db.awaitIndexes(60)", vec![])
        .await
        .expect("awaitIndexes failed");

    let long_term = Arc::new(Neo4jLongTermStore::new(client.clone()));
    let short_term = Arc::new(Neo4jShortTermStore::new(client.clone()));
    let reasoning = Arc::new(Neo4jReasoningStore::new(client.clone()));
    let memory = MemoryClient::new(
        MemoryPorts {
            short_term,
            long_term: long_term.clone(),
            reasoning,
            embedder: None,
            cypher_query: None,
        },
        &MemoryOptions::default(),
    )
    .expect("create MemoryClient failed");

    // =========================================================================
    // Setup: Create target Entity A and Entity B
    // =========================================================================
    eprintln!("[BOLT-07] Creating target entities Entity A and Entity B...");
    let entity_a = memory
        .long_term()
        .add_entity(
            "Entity A",
            "SYSTEM",
            AddEntityOptions {
                generate_embedding: false,
                ..Default::default()
            },
        )
        .await
        .expect("create entity A failed")
        .0;

    let entity_b = memory
        .long_term()
        .add_entity(
            "Entity B",
            "DATABASE",
            AddEntityOptions {
                generate_embedding: false,
                ..Default::default()
            },
        )
        .await
        .expect("create entity B failed")
        .0;

    eprintln!(
        "[BOLT-07] Entity A id: {}, Entity B id: {}",
        entity_a.id, entity_b.id
    );

    // =========================================================================
    // BOLT-07-05: Normative ABOUT relationship (:Preference)-[:ABOUT]->(:Entity)
    // =========================================================================
    eprintln!("[BOLT-07-05] --- Testing normative ABOUT relationship ---");
    let user_about = "user-qa-about";

    // 1. Create a preference linked with about_entities: [entity_a.id]
    let pref_about = memory
        .long_term()
        .add_preference(
            "editor",
            "neovim",
            AddPreferenceOptions {
                user_identifier: Some(user_about.into()),
                about_entities: vec![entity_a.id],
                generate_embedding: false,
                ..Default::default()
            },
        )
        .await
        .expect("add_preference with about_entities failed");

    eprintln!("[BOLT-07-05] Created pref_about: {}", pref_about.id);

    // 2. Validate via direct Cypher:
    // Exactly one edge (p)-[:ABOUT]->(entity_a), correct direction, without properties
    let shape_about_query = "MATCH (p:Preference {id: $pref_id})-[r:ABOUT]->(e:Entity) \
                             RETURN count(r) AS edges, collect(type(r)) AS kinds, \
                             collect(size(keys(r))) AS properties, collect(e.id) AS targets";
    let rows_about = client
        .execute(
            shape_about_query,
            vec![("pref_id", pref_about.id.to_string().into())],
        )
        .await
        .expect("Cypher query for ABOUT edge failed");

    assert_eq!(
        rows_about[0].get::<i64>("edges").unwrap(),
        1,
        "must have exactly 1 ABOUT edge"
    );
    assert_eq!(
        rows_about[0].get::<Vec<String>>("kinds").unwrap(),
        vec!["ABOUT"],
        "edge type must be ABOUT"
    );
    assert_eq!(
        rows_about[0].get::<Vec<i64>>("properties").unwrap(),
        vec![0],
        "ABOUT edge must have 0 properties"
    );
    assert_eq!(
        rows_about[0].get::<Vec<String>>("targets").unwrap(),
        vec![entity_a.id.to_string()],
        "ABOUT edge target must be entity A"
    );

    // Validate direction: no incoming ABOUT edge from Entity A to Preference
    let reverse_about_query =
        "MATCH (e:Entity {id: $entity_id})-[r:ABOUT]->(p:Preference {id: $pref_id}) \
                               RETURN count(r) AS reverse_edges";
    let rev_about_rows = client
        .execute(
            reverse_about_query,
            vec![
                ("pref_id", pref_about.id.to_string().into()),
                ("entity_id", entity_a.id.to_string().into()),
            ],
        )
        .await
        .expect("Cypher query for reverse ABOUT edge failed");
    assert_eq!(
        rev_about_rows[0].get::<i64>("reverse_edges").unwrap(),
        0,
        "no reverse ABOUT edge allowed (must be strictly Preference -> Entity)"
    );

    // 3. Idempotency: re-linking does not duplicate the edge
    eprintln!("[BOLT-07-05] Testing idempotency of ABOUT edge linking...");
    long_term
        .link_preference_about_entity(pref_about.id, entity_a.id)
        .await
        .expect("first re-link of ABOUT entity must succeed");
    long_term
        .link_preference_about_entity(pref_about.id, entity_a.id)
        .await
        .expect("second re-link of ABOUT entity must succeed");

    let idemp_about_rows = client
        .execute(
            "MATCH (p:Preference {id: $pref_id})-[r:ABOUT]->(e:Entity {id: $entity_id}) \
             RETURN count(r) AS edges",
            vec![
                ("pref_id", pref_about.id.to_string().into()),
                ("entity_id", entity_a.id.to_string().into()),
            ],
        )
        .await
        .expect("Cypher query for idempotent ABOUT edge count failed");
    assert_eq!(
        idemp_about_rows[0].get::<i64>("edges").unwrap(),
        1,
        "re-linking ABOUT edge must remain strictly idempotent (exactly 1 edge)"
    );

    // 4. Missing target entity rejection: safe failure, no orphan records or partial writes
    eprintln!("[BOLT-07-05] Testing missing target entity rejection in application layer...");
    let (nodes_before_missing, edges_before_missing) = count_nodes_edges(&client).await;
    let missing_entity_id = Uuid::new_v4();

    let missing_res = memory
        .long_term()
        .add_preference(
            "editor",
            "emacs",
            AddPreferenceOptions {
                user_identifier: Some(user_about.into()),
                about_entities: vec![missing_entity_id],
                generate_embedding: false,
                ..Default::default()
            },
        )
        .await;

    assert!(
        missing_res.is_err(),
        "creating preference with missing about target must fail"
    );
    let err_msg = missing_res.unwrap_err().to_string();
    assert!(
        err_msg.contains("not found: entity") && err_msg.contains(&missing_entity_id.to_string()),
        "error message must clearly indicate missing entity: {err_msg}"
    );

    let (nodes_after_missing, edges_after_missing) = count_nodes_edges(&client).await;
    assert_eq!(
        nodes_after_missing, nodes_before_missing,
        "no orphan nodes created after application-level rejection"
    );
    assert_eq!(
        edges_after_missing, edges_before_missing,
        "no orphan edges created after application-level rejection"
    );

    // Also verify store-level atomic rollback for write_preference with missing about target
    eprintln!("[BOLT-07-05] Testing store-level atomic rollback on missing about entity...");
    let orphaned_pref = Preference {
        id: Uuid::new_v4(),
        category: "editor".into(),
        preference: "nano".into(),
        context: None,
        confidence: 1.0,
        embedding: None,
        valid_from: Some(Utc::now()),
        valid_until: None,
        created_at: Utc::now(),
        metadata: Default::default(),
    };
    let store_res = long_term
        .write_preference(
            &orphaned_pref,
            Some(user_about),
            &[],
            &[missing_entity_id],
            0.9,
        )
        .await;
    assert!(
        store_res.is_err(),
        "store.write_preference must fail with missing about entity"
    );
    let (nodes_after_store, edges_after_store) = count_nodes_edges(&client).await;
    assert_eq!(
        nodes_after_store, nodes_before_missing,
        "store.write_preference must roll back atomically (no orphan nodes)"
    );
    assert_eq!(
        edges_after_store, edges_before_missing,
        "store.write_preference must roll back atomically (no orphan edges)"
    );

    // =========================================================================
    // BOLT-07-06: Observable APPLIES_TO relationship and filtered reading
    // =========================================================================
    eprintln!("[BOLT-07-06] --- Testing observable APPLIES_TO relationship & filtered reads ---");
    let user_applies = "user-qa-applies-pair";

    // 1. Create pref_applies_a with applies_to: [entity_a.id]
    let pref_applies_a = memory
        .long_term()
        .add_preference(
            "language",
            "rust",
            AddPreferenceOptions {
                user_identifier: Some(user_applies.into()),
                applies_to: vec![entity_a.id],
                generate_embedding: false,
                ..Default::default()
            },
        )
        .await
        .expect("add_preference pref_applies_a failed");

    // 2. Create pref_applies_b with applies_to: [entity_b.id]
    let pref_applies_b = memory
        .long_term()
        .add_preference(
            "database",
            "neo4j",
            AddPreferenceOptions {
                user_identifier: Some(user_applies.into()),
                applies_to: vec![entity_b.id],
                generate_embedding: false,
                ..Default::default()
            },
        )
        .await
        .expect("add_preference pref_applies_b failed");

    eprintln!(
        "[BOLT-07-06] Created pref_applies_a ({}) and pref_applies_b ({})",
        pref_applies_a.id, pref_applies_b.id
    );

    // 3. Direct Cypher validation for APPLIES_TO:
    // Validate pref_applies_a -> entity_a
    let shape_applies_query = "MATCH (p:Preference {id: $pref_id})-[r:APPLIES_TO]->(e:Entity) \
                               RETURN count(r) AS edges, collect(type(r)) AS kinds, \
                               collect(size(keys(r))) AS properties, collect(e.id) AS targets";
    let rows_applies_a = client
        .execute(
            shape_applies_query,
            vec![("pref_id", pref_applies_a.id.to_string().into())],
        )
        .await
        .expect("Cypher query for APPLIES_TO edge A failed");
    assert_eq!(
        rows_applies_a[0].get::<i64>("edges").unwrap(),
        1,
        "must have exactly 1 APPLIES_TO edge for pref_applies_a"
    );
    assert_eq!(
        rows_applies_a[0].get::<Vec<String>>("kinds").unwrap(),
        vec!["APPLIES_TO"],
        "edge type must be APPLIES_TO"
    );
    assert_eq!(
        rows_applies_a[0].get::<Vec<i64>>("properties").unwrap(),
        vec![0],
        "APPLIES_TO edge must have 0 properties"
    );
    assert_eq!(
        rows_applies_a[0].get::<Vec<String>>("targets").unwrap(),
        vec![entity_a.id.to_string()],
        "APPLIES_TO target must be entity A"
    );

    // Validate direction: no reverse APPLIES_TO edge from entity_a to pref_applies_a
    let rev_applies_query =
        "MATCH (e:Entity {id: $entity_id})-[r:APPLIES_TO]->(p:Preference {id: $pref_id}) \
                             RETURN count(r) AS reverse_edges";
    let rev_applies_rows = client
        .execute(
            rev_applies_query,
            vec![
                ("pref_id", pref_applies_a.id.to_string().into()),
                ("entity_id", entity_a.id.to_string().into()),
            ],
        )
        .await
        .expect("Cypher query for reverse APPLIES_TO failed");
    assert_eq!(
        rev_applies_rows[0].get::<i64>("reverse_edges").unwrap(),
        0,
        "no reverse APPLIES_TO edge allowed"
    );

    // Validate pref_applies_b -> entity_b
    let rows_applies_b = client
        .execute(
            shape_applies_query,
            vec![("pref_id", pref_applies_b.id.to_string().into())],
        )
        .await
        .expect("Cypher query for APPLIES_TO edge B failed");
    assert_eq!(
        rows_applies_b[0].get::<i64>("edges").unwrap(),
        1,
        "must have exactly 1 APPLIES_TO edge for pref_applies_b"
    );
    assert_eq!(
        rows_applies_b[0].get::<Vec<String>>("kinds").unwrap(),
        vec!["APPLIES_TO"]
    );
    assert_eq!(
        rows_applies_b[0].get::<Vec<i64>>("properties").unwrap(),
        vec![0],
        "APPLIES_TO edge must have 0 properties"
    );
    assert_eq!(
        rows_applies_b[0].get::<Vec<String>>("targets").unwrap(),
        vec![entity_b.id.to_string()],
        "APPLIES_TO target must be entity B"
    );

    // 4. Validate idempotency of APPLIES_TO edge
    eprintln!("[BOLT-07-06] Testing idempotency of APPLIES_TO edge linking...");
    long_term
        .link_preference_to_entity(pref_applies_a.id, entity_a.id)
        .await
        .expect("re-link APPLIES_TO must succeed");
    long_term
        .link_preference_to_entity(pref_applies_a.id, entity_a.id)
        .await
        .expect("second re-link APPLIES_TO must succeed");
    let idemp_applies_rows = client
        .execute(
            "MATCH (p:Preference {id: $pref_id})-[r:APPLIES_TO]->(e:Entity {id: $entity_id}) \
             RETURN count(r) AS edges",
            vec![
                ("pref_id", pref_applies_a.id.to_string().into()),
                ("entity_id", entity_a.id.to_string().into()),
            ],
        )
        .await
        .expect("Cypher query for idempotent APPLIES_TO edge count failed");
    assert_eq!(
        idemp_applies_rows[0].get::<i64>("edges").unwrap(),
        1,
        "re-linking APPLIES_TO edge must remain strictly idempotent"
    );

    // 5. Validating strict filtered reading for the pair:
    // - Consultar con applies_to = Some(entity_a.id) debe devolver exactamente pref_applies_a y excluir pref_applies_b.
    // - Consultar con applies_to = Some(entity_b.id) debe devolver únicamente pref_applies_b.
    // - Consultar sin filtro debe devolver ambas.
    eprintln!("[BOLT-07-06] Validating filtered reading for pref_applies_a and pref_applies_b...");

    // Filter A:
    let filtered_a_app = memory
        .long_term()
        .get_preferences_for_filtered(user_applies, false, Some(entity_a.id), None)
        .await
        .expect("get_preferences_for_filtered for entity_a failed");
    let filtered_a_store = long_term
        .preferences_for_filtered(user_applies, false, Some(entity_a.id), None)
        .await
        .expect("store.preferences_for_filtered for entity_a failed");

    assert_eq!(
        filtered_a_app.len(),
        1,
        "must return exactly 1 preference for entity_a"
    );
    assert_eq!(
        filtered_a_app[0].id, pref_applies_a.id,
        "must return exactly pref_applies_a and exclude pref_applies_b"
    );
    assert_eq!(
        filtered_a_store.len(),
        1,
        "store must return exactly 1 preference for entity_a"
    );
    assert_eq!(
        filtered_a_store[0].id, pref_applies_a.id,
        "store must return exactly pref_applies_a"
    );

    // Filter B:
    let filtered_b_app = memory
        .long_term()
        .get_preferences_for_filtered(user_applies, false, Some(entity_b.id), None)
        .await
        .expect("get_preferences_for_filtered for entity_b failed");
    let filtered_b_store = long_term
        .preferences_for_filtered(user_applies, false, Some(entity_b.id), None)
        .await
        .expect("store.preferences_for_filtered for entity_b failed");

    assert_eq!(
        filtered_b_app.len(),
        1,
        "must return exactly 1 preference for entity_b"
    );
    assert_eq!(
        filtered_b_app[0].id, pref_applies_b.id,
        "must return únicamente pref_applies_b and exclude pref_applies_a"
    );
    assert_eq!(
        filtered_b_store.len(),
        1,
        "store must return exactly 1 preference for entity_b"
    );
    assert_eq!(
        filtered_b_store[0].id, pref_applies_b.id,
        "store must return únicamente pref_applies_b"
    );

    // Unfiltered (must return both):
    let unfiltered_app = memory
        .long_term()
        .get_preferences_for(user_applies, false)
        .await
        .expect("get_preferences_for without filter failed");
    let unfiltered_store = long_term
        .preferences_for(user_applies, false)
        .await
        .expect("store.preferences_for without filter failed");

    assert_eq!(
        unfiltered_app.len(),
        2,
        "unfiltered query must return both preferences"
    );
    assert_eq!(
        unfiltered_store.len(),
        2,
        "unfiltered store query must return both preferences"
    );
    let unfiltered_ids: Vec<Uuid> = unfiltered_app.iter().map(|p| p.id).collect();
    assert!(unfiltered_ids.contains(&pref_applies_a.id));
    assert!(unfiltered_ids.contains(&pref_applies_b.id));

    // 6. Coexistence of BOTH ABOUT and APPLIES_TO relationships
    eprintln!(
        "[BOLT-07-06] Testing coexistence of ABOUT and APPLIES_TO on distinct and same entities..."
    );
    let user_coexist = "user-qa-coexist";

    // Case A: Preference with ABOUT -> entity_a AND APPLIES_TO -> entity_b (distinct entities)
    let pref_dual_distinct = memory
        .long_term()
        .add_preference(
            "architecture",
            "hexagonal",
            AddPreferenceOptions {
                user_identifier: Some(user_coexist.into()),
                about_entities: vec![entity_a.id],
                applies_to: vec![entity_b.id],
                generate_embedding: false,
                ..Default::default()
            },
        )
        .await
        .expect("add_preference pref_dual_distinct failed");

    let dual_distinct_rows = client
        .execute(
            "MATCH (p:Preference {id: $id})-[r]->(e:Entity) \
             RETURN type(r) AS rel_type, e.id AS target_id",
            vec![("id", pref_dual_distinct.id.to_string().into())],
        )
        .await
        .unwrap();
    assert_eq!(
        dual_distinct_rows.len(),
        2,
        "pref_dual_distinct must have exactly 2 outgoing edges"
    );
    let mut found_about_a = false;
    let mut found_applies_b = false;
    for r in dual_distinct_rows {
        let rel_type = r.get::<String>("rel_type").unwrap();
        let target_id = r.get::<String>("target_id").unwrap();
        if rel_type == "ABOUT" && target_id == entity_a.id.to_string() {
            found_about_a = true;
        }
        if rel_type == "APPLIES_TO" && target_id == entity_b.id.to_string() {
            found_applies_b = true;
        }
    }
    assert!(found_about_a, "ABOUT edge to entity_a must coexist");
    assert!(found_applies_b, "APPLIES_TO edge to entity_b must coexist");

    // Case B: Preference with BOTH ABOUT -> entity_a AND APPLIES_TO -> entity_a (same entity)
    let pref_dual_same = memory
        .long_term()
        .add_preference(
            "testing",
            "strict_tdd",
            AddPreferenceOptions {
                user_identifier: Some(user_coexist.into()),
                about_entities: vec![entity_a.id],
                applies_to: vec![entity_a.id],
                generate_embedding: false,
                ..Default::default()
            },
        )
        .await
        .expect("add_preference pref_dual_same failed");

    let dual_same_rows = client
        .execute(
            "MATCH (p:Preference {id: $id})-[r]->(e:Entity {id: $entity_id}) \
             RETURN collect(type(r)) AS rel_types",
            vec![
                ("id", pref_dual_same.id.to_string().into()),
                ("entity_id", entity_a.id.to_string().into()),
            ],
        )
        .await
        .unwrap();
    let mut same_rel_types: Vec<String> = dual_same_rows[0].get("rel_types").unwrap();
    same_rel_types.sort();
    assert_eq!(
        same_rel_types,
        vec!["ABOUT", "APPLIES_TO"],
        "both ABOUT and APPLIES_TO must coexist pointing to the same entity without replacement"
    );

    // Validate filtered reading in coexistence scope:
    // - Filter applies_to = entity_a: returns pref_dual_same, excludes pref_dual_distinct
    let coexist_filtered_a = memory
        .long_term()
        .get_preferences_for_filtered(user_coexist, false, Some(entity_a.id), None)
        .await
        .expect("coexist filtered for entity_a failed");
    assert_eq!(coexist_filtered_a.len(), 1);
    assert_eq!(coexist_filtered_a[0].id, pref_dual_same.id);

    // - Filter applies_to = entity_b: returns pref_dual_distinct, excludes pref_dual_same
    let coexist_filtered_b = memory
        .long_term()
        .get_preferences_for_filtered(user_coexist, false, Some(entity_b.id), None)
        .await
        .expect("coexist filtered for entity_b failed");
    assert_eq!(coexist_filtered_b.len(), 1);
    assert_eq!(coexist_filtered_b[0].id, pref_dual_distinct.id);

    // - Unfiltered: returns both pref_dual_distinct and pref_dual_same
    let coexist_unfiltered = memory
        .long_term()
        .get_preferences_for(user_coexist, false)
        .await
        .expect("coexist unfiltered failed");
    assert_eq!(coexist_unfiltered.len(), 2);

    eprintln!("=== [SUCCESS] BOLT-07 Preference Relationships Test Completed Successfully ===");
}
