//! Consolidation Cypher statements, ported from upstream
//! `memory/consolidation.py` (neo4j-labs/agent-memory @ 9d45af55, Apache-2.0)
//! with explicit owner-scope preservation for preference supersession.

/// Find entity pairs of the same type whose embeddings exceed `$threshold`
/// cosine similarity and are not already linked by `:SAME_AS`.
pub const FIND_DUPLICATE_ENTITY_PAIRS: &str = "
MATCH (e1:Entity)
WHERE e1.embedding IS NOT NULL
CALL {
    WITH e1
    CALL db.index.vector.queryNodes(
        'entity_embedding_idx', 5, e1.embedding
    ) YIELD node AS e2, score
    WHERE e2 <> e1
      AND e2.type = e1.type
      AND score >= $threshold
      AND NOT (e1)-[:SAME_AS]-(e2)
      AND e1.id < e2.id
    RETURN e2, score
}
RETURN e1.id AS a_id, e1.name AS a_name,
       e2.id AS b_id, e2.name AS b_name,
       score
ORDER BY score DESC, a_id ASC, b_id ASC
LIMIT $limit
";

/// Link a duplicate entity pair with `:SAME_AS` (`status = 'auto_consolidated'`).
pub const LINK_CONSOLIDATED_ENTITY_PAIR: &str = "
MATCH (a:Entity {id: $a_id})
MATCH (b:Entity {id: $b_id})
WHERE NOT (a)-[:SAME_AS]-(b)
MERGE (a)-[r:SAME_AS]->(b)
ON CREATE SET r.confidence = $score,
              r.match_type = 'embedding',
              r.status = 'auto_consolidated',
              r.created_at = datetime()
RETURN count(r) AS created
";

/// Find reasoning traces with at least `$min_steps` steps that are neither
/// summarized nor already flagged for summarization.
pub const FIND_LONG_TRACES: &str = "
MATCH (rt:ReasoningTrace)-[:HAS_STEP]->(s:ReasoningStep)
WITH rt, count(s) AS step_count
WHERE step_count >= $min_steps
  AND coalesce(rt.summarization_pending, false) = false
  AND coalesce(rt.has_summary, false) = false
RETURN rt.id AS trace_id, rt.task AS task, step_count
ORDER BY step_count DESC, trace_id ASC
LIMIT $limit
";

/// Flag long reasoning traces for out-of-band summarization.
pub const FLAG_LONG_TRACES_FOR_SUMMARY: &str = "
UNWIND $trace_ids AS trace_id
MATCH (rt:ReasoningTrace {id: trace_id})
WHERE coalesce(rt.summarization_pending, false) = false
  AND coalesce(rt.has_summary, false) = false
SET rt.summarization_pending = true,
    rt.flagged_for_summary_at = datetime()
RETURN count(rt) AS flagged
";

/// Find newer preferences in the same category and owner namespace with high
/// embedding similarity that likely supersede an older active preference.
///
/// Comparing `old_owners = new_owners` enforces strict tenant isolation: a
/// preference owned by one user can never supersede another user's preference
/// or an unowned preference.
pub const FIND_SUPERSEDED_PREFERENCES: &str = "
MATCH (old:Preference)
WHERE old.embedding IS NOT NULL
  AND NOT (old)-[:SUPERSEDED_BY]->()
CALL {
    WITH old
    CALL db.index.vector.queryNodes(
        'preference_embedding_idx', 5, old.embedding
    ) YIELD node AS new, score
    WHERE new <> old
      AND new.category = old.category
      AND new.preference <> old.preference
      AND score >= $threshold
      AND coalesce(new.created_at, datetime()) > coalesce(old.created_at, datetime())
    RETURN new, score
}
OPTIONAL MATCH (u_old:User)-[:HAS_PREFERENCE]->(old)
OPTIONAL MATCH (u_new:User)-[:HAS_PREFERENCE]->(new)
WITH old, new, score,
     collect(DISTINCT u_old.identifier) AS old_owners,
     collect(DISTINCT u_new.identifier) AS new_owners
WHERE old_owners = new_owners
RETURN old.id AS old_id, old.preference AS old_text,
       new.id AS new_id, new.preference AS new_text,
       score
ORDER BY score DESC, old_id ASC, new_id ASC
LIMIT $limit
";

/// Mark an older preference as superseded by a newer preference.
pub const APPLY_SUPERSEDE_PREFERENCE: &str = "
MATCH (old:Preference {id: $old_id})
MATCH (new:Preference {id: $new_id})
WHERE NOT (old)-[:SUPERSEDED_BY]->()
OPTIONAL MATCH (u_old:User)-[:HAS_PREFERENCE]->(old)
WITH old, new, collect(DISTINCT u_old) AS old_owners
OPTIONAL MATCH (u_new:User)-[:HAS_PREFERENCE]->(new)
WITH old, new, old_owners, collect(DISTINCT u_new) AS new_owners
WHERE size(old_owners) = size(new_owners)
  AND all(owner IN old_owners WHERE owner IN new_owners)
MERGE (old)-[:SUPERSEDED_BY]->(new)
SET old.valid_until = coalesce(old.valid_until, datetime())
RETURN count(old) AS updated
";

/// Find non-archived conversations older than `$ttl_days`.
pub const FIND_EXPIRED_CONVERSATIONS: &str = "
MATCH (c:Conversation)
WHERE coalesce(c.archived, false) = false
  AND coalesce(c.created_at, c.updated_at) < datetime() - duration({days: $ttl_days})
RETURN c.id AS id, c.session_id AS session_id,
       coalesce(c.created_at, c.updated_at) AS created_at
ORDER BY created_at ASC, id ASC
LIMIT $limit
";

/// Mark expired conversations as archived without deleting data.
pub const ARCHIVE_CONVERSATIONS: &str = "
UNWIND $ids AS id
MATCH (c:Conversation {id: id})
WHERE coalesce(c.archived, false) = false
SET c.archived = true,
    c.archived_at = datetime()
RETURN count(c) AS archived
";

/// Persist a `:ConsolidationRun` audit node.
pub const CREATE_CONSOLIDATION_RUN: &str = "
CREATE (cr:ConsolidationRun {
    id: $id,
    kind: $kind,
    started_at: datetime(),
    completed_at: datetime(),
    stats_json: $stats_json
})
RETURN cr.id AS id
";

/// Persist an explicit `:MemoryReadAudit` node (`BOLT-08-11`).
pub const CREATE_MEMORY_READ_AUDIT: &str = "
CREATE (a:MemoryReadAudit {
    id: $audit_id,
    kind: $kind,
    query: $query,
    user_identifier: $user_identifier,
    result_count: $result_count,
    metadata_json: $metadata_json,
    recorded_at: datetime()
})
RETURN a.id AS id
";

/// Link an existing `:MemoryReadAudit` node to its `:User` via `:PERFORMED_READ`.
pub const LINK_USER_TO_READ_AUDIT: &str = "
MERGE (u:User {identifier: $user_identifier})
ON CREATE SET u.id = $user_identifier, u.created_at = datetime()
WITH u
MATCH (a:MemoryReadAudit {id: $audit_id})
MERGE (u)-[:PERFORMED_READ]->(a)
RETURN a.id AS id
";
