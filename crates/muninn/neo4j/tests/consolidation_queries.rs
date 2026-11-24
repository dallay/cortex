use muninn_neo4j::queries::consolidation::*;

#[test]
fn dedupe_queries_use_canonical_vector_index_and_same_as_edge() {
    assert!(FIND_DUPLICATE_ENTITY_PAIRS.contains("'entity_embedding_idx'"));
    assert!(FIND_DUPLICATE_ENTITY_PAIRS.contains("NOT (e1)-[:SAME_AS]-(e2)"));
    assert!(FIND_DUPLICATE_ENTITY_PAIRS.contains("e1.id < e2.id"));
    assert!(LINK_CONSOLIDATED_ENTITY_PAIR.contains("MERGE (a)-[r:SAME_AS]->(b)"));
    assert!(LINK_CONSOLIDATED_ENTITY_PAIR.contains("r.status = 'auto_consolidated'"));
}

#[test]
fn preference_supersession_enforces_same_owner_scope() {
    assert!(FIND_SUPERSEDED_PREFERENCES.contains("'preference_embedding_idx'"));
    assert!(FIND_SUPERSEDED_PREFERENCES.contains("HAS_PREFERENCE"));
    assert!(FIND_SUPERSEDED_PREFERENCES.contains("WHERE old_owners = new_owners"));
    assert!(!FIND_SUPERSEDED_PREFERENCES.contains("ABOUT"));
    assert!(APPLY_SUPERSEDE_PREFERENCE.contains("size(old_owners) = size(new_owners)"));
    assert!(
        APPLY_SUPERSEDE_PREFERENCE.contains("all(owner IN old_owners WHERE owner IN new_owners)")
    );
    assert!(APPLY_SUPERSEDE_PREFERENCE.contains("MERGE (old)-[:SUPERSEDED_BY]->(new)"));
    assert!(APPLY_SUPERSEDE_PREFERENCE.contains("old.valid_until"));
}

#[test]
fn audit_queries_target_canonical_labels() {
    assert!(CREATE_CONSOLIDATION_RUN.contains("CREATE (cr:ConsolidationRun"));
    assert!(CREATE_CONSOLIDATION_RUN.contains("stats_json: $stats_json"));
    assert!(CREATE_MEMORY_READ_AUDIT.contains("CREATE (a:MemoryReadAudit"));
    assert!(LINK_USER_TO_READ_AUDIT.contains("MERGE (u)-[:PERFORMED_READ]->(a)"));
}
