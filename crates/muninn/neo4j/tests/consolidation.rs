//! BOLT-08-05 & BOLT-08-11: persistent consolidation hygiene jobs and
//! explicit audit nodes (`:ConsolidationRun`, `:MemoryReadAudit`) against Neo4j.
use std::sync::Arc;

use muninn_application::{MemoryClient, MemoryOptions, MemoryPorts};
use muninn_core::ports::ReadAuditRecord;
use muninn_neo4j::{
    Neo4jClient, Neo4jConsolidationStore, Neo4jLongTermStore, Neo4jReasoningStore,
    Neo4jShortTermStore, SchemaManager,
};
use neo4rs::BoltType;
use serde_json::Value;

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
async fn consolidation_persists_mutations_and_audit_nodes() {
    let Some(fixture) = DockerFixture::start("MUNINN_CONSOLIDATION_URI") else {
        return;
    };
    let client = fixture.connect_fresh().await;
    SchemaManager::new(&client, 3).setup_all().await.unwrap();

    let consolidation_store = Arc::new(Neo4jConsolidationStore::new(client.clone()));
    let memory = MemoryClient::new(
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
    .with_consolidation(consolidation_store);
    let consolidation = memory.consolidation().expect("consolidation wired");

    // 1. Seed entities: two near-identical PERSON entities and one unrelated LOCATION.
    client
        .execute(
            "CREATE (:Entity {id: 'ent-1', name: 'Ada Lovelace', type: 'PERSON', embedding: [1.0, 0.0, 0.0]}),
                    (:Entity {id: 'ent-2', name: 'A. Lovelace', type: 'PERSON', embedding: [0.99, 0.14, 0.0]}),
                    (:Entity {id: 'ent-3', name: 'London', type: 'LOCATION', embedding: [1.0, 0.0, 0.0]})",
            vec![],
        )
        .await
        .unwrap();

    // 2. Seed traces: one long trace (3 steps) and one short trace (1 step).
    client
        .execute(
            "CREATE (long:ReasoningTrace {id: 'trace-long', task: 'deep analysis', started_at: datetime()})
             CREATE (long)-[:HAS_STEP]->(:ReasoningStep {id: 's1', step_number: 1})
             CREATE (long)-[:HAS_STEP]->(:ReasoningStep {id: 's2', step_number: 2})
             CREATE (long)-[:HAS_STEP]->(:ReasoningStep {id: 's3', step_number: 3})
             CREATE (short:ReasoningTrace {id: 'trace-short', task: 'quick check', started_at: datetime()})
             CREATE (short)-[:HAS_STEP]->(:ReasoningStep {id: 's4', step_number: 1})",
            vec![],
        )
        .await
        .unwrap();

    // 3. Seed preferences:
    //    - owner-a has old + newer similar preference in same category (must supersede)
    //    - owner-b has an even newer similar preference in same category (must NOT supersede owner-a!)
    client
        .execute(
            "CREATE (ua:User {id: 'owner-a', identifier: 'owner-a'})
             CREATE (ub:User {id: 'owner-b', identifier: 'owner-b'})
             CREATE (p_old:Preference {
                 id: 'pref-a-old',
                 category: 'ui',
                 preference: 'Prefer dark theme',
                 embedding: [0.0, 1.0, 0.0],
                 created_at: datetime('2026-01-01T00:00:00Z')
             })
             CREATE (p_new:Preference {
                 id: 'pref-a-new',
                 category: 'ui',
                 preference: 'Prefer dark mode always',
                 embedding: [0.05, 0.998, 0.0],
                 created_at: datetime('2026-02-01T00:00:00Z')
             })
             CREATE (p_other:Preference {
                 id: 'pref-b-new',
                 category: 'ui',
                 preference: 'Prefer midnight dark mode',
                 embedding: [0.02, 0.999, 0.0],
                 created_at: datetime('2026-03-01T00:00:00Z')
             })
             CREATE (ua)-[:HAS_PREFERENCE]->(p_old)
             CREATE (ua)-[:HAS_PREFERENCE]->(p_new)
             CREATE (ub)-[:HAS_PREFERENCE]->(p_other)",
            vec![],
        )
        .await
        .unwrap();

    // 4. Seed conversations: one expired (120 days old) and one fresh (1 day old).
    client
        .execute(
            "CREATE (:Conversation {
                 id: 'conv-old',
                 session_id: 'sess-old',
                 owner_scope: 'unowned',
                 created_at: datetime() - duration({days: 120}),
                 updated_at: datetime() - duration({days: 120})
             }),
             (:Conversation {
                 id: 'conv-fresh',
                 session_id: 'sess-fresh',
                 owner_scope: 'unowned',
                 created_at: datetime() - duration({days: 1}),
                 updated_at: datetime() - duration({days: 1})
             })",
            vec![],
        )
        .await
        .unwrap();

    // Wait briefly for Neo4j vector indexes to populate the newly created nodes.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let before_dry_run = graph_snapshot(&client).await;

    // --- Dry-run checks: must find candidates and make zero graph mutations ---
    let dedupe_dry = consolidation
        .dedupe_entities(Some(0.95), Some(10), Some(true))
        .await
        .unwrap();
    assert!(dedupe_dry.dry_run);
    assert_eq!(dedupe_dry.candidates_found, 1);
    assert_eq!(dedupe_dry.mutations_applied, 0);
    assert!(dedupe_dry.run_id.is_none());

    let traces_dry = consolidation
        .summarize_long_traces(Some(3), Some(10), Some(true))
        .await
        .unwrap();
    assert!(traces_dry.dry_run);
    assert_eq!(traces_dry.candidates_found, 1);
    assert_eq!(traces_dry.candidates[0].id, "trace-long");
    assert_eq!(traces_dry.mutations_applied, 0);
    assert!(traces_dry.run_id.is_none());

    let prefs_dry = consolidation
        .detect_superseded_preferences(Some(0.95), Some(10), Some(true))
        .await
        .unwrap();
    assert!(prefs_dry.dry_run);
    assert_eq!(prefs_dry.candidates_found, 1);
    assert_eq!(prefs_dry.candidates[0].id, "pref-a-old");
    assert_eq!(
        prefs_dry.candidates[0].target_id.as_deref(),
        Some("pref-a-new")
    );
    assert_eq!(prefs_dry.mutations_applied, 0);
    assert!(prefs_dry.run_id.is_none());

    let archive_dry = consolidation
        .archive_expired_conversations(Some(90), Some(10), Some(true))
        .await
        .unwrap();
    assert!(archive_dry.dry_run);
    assert_eq!(archive_dry.candidates_found, 1);
    assert_eq!(archive_dry.candidates[0].id, "conv-old");
    assert_eq!(archive_dry.mutations_applied, 0);
    assert!(archive_dry.run_id.is_none());

    assert_eq!(
        graph_snapshot(&client).await,
        before_dry_run,
        "dry_run consolidation jobs must not mutate the graph"
    );

    // --- Execute mode: verify real graph effects and :ConsolidationRun nodes ---
    let dedupe_exec = consolidation
        .dedupe_entities(Some(0.95), Some(10), Some(false))
        .await
        .unwrap();
    assert!(!dedupe_exec.dry_run);
    assert_eq!(dedupe_exec.mutations_applied, 1);
    let dedupe_run_id = dedupe_exec.run_id.expect("dedupe run_id persisted");

    let same_as = client
        .execute(
            "MATCH (a:Entity {id: 'ent-1'})-[r:SAME_AS]->(b:Entity {id: 'ent-2'})
             RETURN r.status AS status, r.match_type AS match_type, r.confidence AS confidence",
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(same_as.len(), 1);
    assert_eq!(
        same_as[0].get::<String>("status").unwrap(),
        "auto_consolidated"
    );
    assert_eq!(same_as[0].get::<String>("match_type").unwrap(), "embedding");
    assert!(same_as[0].get::<f64>("confidence").unwrap() >= 0.95);

    // Idempotency: re-running dedupe_entities finds 0 candidates and creates no new run.
    let dedupe_second = consolidation
        .dedupe_entities(Some(0.95), Some(10), Some(false))
        .await
        .unwrap();
    assert_eq!(dedupe_second.candidates_found, 0);
    assert_eq!(dedupe_second.mutations_applied, 0);
    assert!(dedupe_second.run_id.is_none());

    let traces_exec = consolidation
        .summarize_long_traces(Some(3), Some(10), Some(false))
        .await
        .unwrap();
    assert_eq!(traces_exec.mutations_applied, 1);
    assert!(traces_exec.run_id.is_some());
    let flagged = client
        .execute(
            "MATCH (rt:ReasoningTrace {id: 'trace-long'})
             RETURN rt.summarization_pending AS pending, rt.flagged_for_summary_at IS NOT NULL AS has_ts",
            vec![],
        )
        .await
        .unwrap();
    assert!(flagged[0].get::<bool>("pending").unwrap());
    assert!(flagged[0].get::<bool>("has_ts").unwrap());

    let prefs_exec = consolidation
        .detect_superseded_preferences(Some(0.95), Some(10), Some(false))
        .await
        .unwrap();
    assert_eq!(prefs_exec.mutations_applied, 1);
    assert!(prefs_exec.run_id.is_some());
    let superseded = client
        .execute(
            "MATCH (old:Preference {id: 'pref-a-old'})-[:SUPERSEDED_BY]->(new:Preference)
             RETURN new.id AS new_id, old.valid_until IS NOT NULL AS has_valid_until",
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(superseded.len(), 1);
    assert_eq!(superseded[0].get::<String>("new_id").unwrap(), "pref-a-new");
    assert!(superseded[0].get::<bool>("has_valid_until").unwrap());

    // Confirm owner-b's preference was never linked to owner-a's preferences.
    let cross_owner = client
        .execute(
            "MATCH (:Preference {id: 'pref-a-old'})-[:SUPERSEDED_BY]->(:Preference {id: 'pref-b-new'})
             RETURN count(*) AS leaks",
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(cross_owner[0].get::<i64>("leaks").unwrap(), 0);

    let archive_exec = consolidation
        .archive_expired_conversations(Some(90), Some(10), Some(false))
        .await
        .unwrap();
    assert_eq!(archive_exec.mutations_applied, 1);
    assert!(archive_exec.run_id.is_some());
    let archived = client
        .execute(
            "MATCH (c:Conversation)
             RETURN c.id AS id, coalesce(c.archived, false) AS archived
             ORDER BY c.id ASC",
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(archived.len(), 2);
    assert_eq!(archived[0].get::<String>("id").unwrap(), "conv-fresh");
    assert!(!archived[0].get::<bool>("archived").unwrap());
    assert_eq!(archived[1].get::<String>("id").unwrap(), "conv-old");
    assert!(archived[1].get::<bool>("archived").unwrap());

    // Verify all 4 :ConsolidationRun audit nodes were persisted with valid JSON stats.
    let runs = client
        .execute(
            "MATCH (cr:ConsolidationRun)
             RETURN cr.id AS id, cr.kind AS kind, cr.stats_json AS stats_json
             ORDER BY cr.kind ASC",
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(runs.len(), 4);
    let dedupe_row = runs
        .iter()
        .find(|row| row.get::<String>("id").unwrap() == dedupe_run_id)
        .expect("dedupe ConsolidationRun persisted");
    assert_eq!(dedupe_row.get::<String>("kind").unwrap(), "dedupe_entities");
    let parsed_stats: Value =
        serde_json::from_str(&dedupe_row.get::<String>("stats_json").unwrap()).unwrap();
    assert_eq!(parsed_stats["applied"], 1);

    // --- BOLT-08-11: explicit :MemoryReadAudit persistence ---
    let mut owned_audit = ReadAuditRecord::new("list preferences for owner-a");
    owned_audit.user_identifier = Some("owner-a".to_owned());
    owned_audit.kind = "preference.read".to_owned();
    owned_audit.result_count = Some(2);
    owned_audit.metadata_json = Some(r#"{"endpoint":"preferences"}"#.to_owned());
    let owned_audit_id = consolidation.record_read_audit(&owned_audit).await.unwrap();

    let audit_rows = client
        .execute(
            "MATCH (u:User {identifier: 'owner-a'})-[:PERFORMED_READ]->(a:MemoryReadAudit {id: $id})
             RETURN a.kind AS kind, a.query AS query, a.result_count AS result_count",
            vec![("id", owned_audit_id.into())],
        )
        .await
        .unwrap();
    assert_eq!(audit_rows.len(), 1);
    assert_eq!(
        audit_rows[0].get::<String>("kind").unwrap(),
        "preference.read"
    );
    assert_eq!(
        audit_rows[0].get::<String>("query").unwrap(),
        "list preferences for owner-a"
    );
    assert_eq!(audit_rows[0].get::<i64>("result_count").unwrap(), 2);

    // Unowned read audit creates node without PERFORMED_READ edge.
    let unowned_audit = ReadAuditRecord::new("graph.export");
    let unowned_audit_id = consolidation
        .record_read_audit(&unowned_audit)
        .await
        .unwrap();
    let unowned_rows = client
        .execute(
            "MATCH (a:MemoryReadAudit {id: $id})
             OPTIONAL MATCH (u:User)-[:PERFORMED_READ]->(a)
             RETURN a.kind AS kind, count(u) AS users",
            vec![("id", unowned_audit_id.into())],
        )
        .await
        .unwrap();
    assert_eq!(unowned_rows.len(), 1);
    assert_eq!(
        unowned_rows[0].get::<String>("kind").unwrap(),
        "memory.read"
    );
    assert_eq!(unowned_rows[0].get::<i64>("users").unwrap(), 0);
}
