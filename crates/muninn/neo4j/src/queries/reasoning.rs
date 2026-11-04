//! Reasoning memory Cypher, ported from upstream `graph/queries.py` and the
//! inline statements of `memory/reasoning.py` (neo4j-labs/agent-memory @
//! 9d45af55, Apache-2.0).
//!
//! Statements without an upstream counterpart (the hosted conversation view)
//! only read or write the upstream graph shape: `HAS_TRACE`, `HAS_STEP`,
//! `USES_TOOL`, `INSTANCE_OF` and `TOUCHED`.

pub const CREATE_REASONING_TRACE: &str = "
OPTIONAL MATCH (c:Conversation {owner_scope: $owner_scope})-[:HAS_MESSAGE]->(m:Message {id: $message_id})
WITH m WHERE $message_id IS NULL OR m IS NOT NULL
CREATE (rt:ReasoningTrace {
    user_identifier: $user_identifier,
    id: $id,
    session_id: $session_id,
    task: $task,
    task_embedding: $task_embedding,
    outcome: $outcome,
    success: $success,
    started_at: datetime(),
    completed_at: $completed_at,
    metadata: $metadata
})
FOREACH (_ IN CASE WHEN m IS NULL THEN [] ELSE [1] END | MERGE (rt)-[:INITIATED_BY]->(m))
FOREACH (_ IN CASE WHEN $user_identifier IS NULL THEN [] ELSE [1] END |
    MERGE (u:User {identifier: $user_identifier}) MERGE (u)-[:HAS_TRACE]->(rt))
RETURN rt
";

/// Inline statement of upstream `start_trace(user_identifier=...)`.
pub const LINK_USER_TO_TRACE: &str = "
MERGE (u:User {identifier: $user_identifier})
ON CREATE SET u.id = $user_identifier, u.created_at = datetime()
WITH u
MATCH (rt:ReasoningTrace {id: $trace_id})
MERGE (u)-[:HAS_TRACE]->(rt)
SET rt.user_identifier = $user_identifier
";

pub const UPDATE_REASONING_TRACE: &str = "
MATCH (rt:ReasoningTrace {id: $id})
SET rt.outcome = $outcome,
    rt.success = $success,
    rt.completed_at = datetime()
RETURN rt
";

/// Inline structured completion writes in pinned `memory/reasoning.py`.
/// These are top-level properties, not entries in the legacy metadata JSON.
pub const SET_TRACE_OUTCOME_FIELDS: &str = "
MATCH (rt:ReasoningTrace {id: $id})
SET rt.error_kind = $error_kind, rt.metrics_json = $metrics_json
";

pub const LAST_TRACE_STEP: &str = "
MATCH (:ReasoningTrace {id: $trace_id})-[:HAS_STEP]->(s:ReasoningStep)
RETURN s.id AS step_id
ORDER BY s.step_number DESC
LIMIT 1
";

/// Upstream `CREATE_REASONING_STEP`, assigning `step_number` in the same
/// statement.
///
/// Upstream reads `count(steps) + 1` in a separate query, so concurrent
/// `add_step` calls can share a number. Here the trace node is write-locked
/// first (`SET`/`REMOVE` of a scratch property holds the lock until commit)
/// and the number is `max(step_number) + 1`, which equals upstream's count
/// while steps are never deleted and stays monotonic if they are.
pub const CREATE_REASONING_STEP: &str = "
MATCH (rt:ReasoningTrace {id: $trace_id})
SET rt._muninn_lock = true
REMOVE rt._muninn_lock
WITH rt
OPTIONAL MATCH (rt)-[:HAS_STEP]->(existing:ReasoningStep)
WITH rt, coalesce(max(existing.step_number), 0) + 1 AS step_number
CREATE (rs:ReasoningStep {
    id: $id,
    step_number: step_number,
    thought: $thought,
    action: $action,
    observation: $observation,
    embedding: $embedding,
    timestamp: datetime(),
    metadata: $metadata
})
CREATE (rt)-[:HAS_STEP {order: step_number}]->(rs)
RETURN rs
";

/// Inline statement of upstream `record_tool_call(auto_observation=True)`.
pub const SET_STEP_OBSERVATION: &str = "
MATCH (s:ReasoningStep {id: $id}) SET s.observation = $observation
";

/// Inline statement of upstream `_generate_step_embeddings_batch`.
pub const GET_STEPS_WITHOUT_EMBEDDING: &str = "
MATCH (rt:ReasoningTrace {id: $id})-[:HAS_STEP]->(s:ReasoningStep)
WHERE s.embedding IS NULL
RETURN s
ORDER BY s.step_number ASC
";

pub const SET_STEP_EMBEDDING: &str = "
MATCH (s:ReasoningStep {id: $id}) SET s.embedding = $embedding
";

/// Upstream `CREATE_TOOL_CALL`, including the incremental `Tool` statistics.
///
/// SOURCE-CONFLICT: the node's `failed_calls` counter only counts `error`
/// and `timeout`, while upstream `get_tool_stats` (and TCK SPEC-4.7.1)
/// report every non-successful call, `failure` included, as failed. The
/// counter is kept verbatim for graph parity; reported stats derive
/// `failed_calls` from `total_calls - successful_calls` as upstream does.
pub const CREATE_TOOL_CALL: &str = "
MATCH (rt:ReasoningTrace)-[:HAS_STEP]->(rs:ReasoningStep {id: $step_id})
OPTIONAL MATCH (:Conversation {owner_scope: $owner_scope})-[:HAS_MESSAGE]->(m:Message {id: $message_id})
WITH rt, rs, m WHERE ($message_id IS NULL OR m IS NOT NULL)
AND (rt.user_identifier = $user_identifier OR (rt.user_identifier IS NULL AND $user_identifier IS NULL))
MERGE (t:Tool {name: $tool_name})
ON CREATE SET t.created_at = datetime(),
              t.total_calls = 0,
              t.successful_calls = 0,
              t.failed_calls = 0,
              t.total_duration_ms = 0
CREATE (tc:ToolCall {
    id: $id,
    tool_name: $tool_name,
    arguments: $arguments,
    result: $result,
    status: $status,
    duration_ms: $duration_ms,
    error: $error,
    timestamp: datetime()
})
CREATE (rs)-[:USES_TOOL]->(tc)
CREATE (tc)-[:INSTANCE_OF]->(t)
WITH t, tc, m
SET t.total_calls = coalesce(t.total_calls, 0) + 1,
    t.successful_calls = coalesce(t.successful_calls, 0) + CASE WHEN tc.status = 'success' THEN 1 ELSE 0 END,
    t.failed_calls = coalesce(t.failed_calls, 0) + CASE WHEN tc.status IN ['error', 'timeout'] THEN 1 ELSE 0 END,
    t.total_duration_ms = coalesce(t.total_duration_ms, 0) + coalesce(tc.duration_ms, 0),
    t.last_used_at = datetime()
FOREACH (_ IN CASE WHEN m IS NULL THEN [] ELSE [1] END | MERGE (tc)-[:TRIGGERED_BY]->(m))
RETURN tc
";

/// Pre-aggregated `Tool` statistics.
///
/// Upstream `GET_TOOL_STATS` reads the same counters but returns only
/// `success_rate`, filters by name client-side and drops `last_used_at`.
/// This variant returns the raw counters (so counts are exact) and filters
/// in Cypher.
pub const GET_TOOL_STATS: &str = "
MATCH (t:Tool)
WHERE $tool_name IS NULL OR t.name = $tool_name
RETURN t.name AS name,
       t.description AS description,
       coalesce(t.total_calls, 0) AS total_calls,
       coalesce(t.successful_calls, 0) AS successful_calls,
       coalesce(t.total_duration_ms, 0) AS total_duration_ms,
       t.last_used_at AS last_used_at
ORDER BY total_calls DESC, name ASC
";

pub const SEARCH_TRACES_BY_EMBEDDING: &str = "
CALL db.index.vector.queryNodes('task_embedding_idx', $limit, $embedding)
YIELD node, score
WHERE score >= $threshold AND ($success_only = false OR node.success = true)
RETURN node AS rt, score
ORDER BY score DESC
";

/// Upstream `SEARCH_STEPS_BY_EMBEDDING`, also returning `rt.id` instead of
/// resolving each hit's trace with a second query.
pub const SEARCH_STEPS_BY_EMBEDDING: &str = "
CALL db.index.vector.queryNodes('step_embedding_idx', $limit, $embedding)
YIELD node AS rs, score
WHERE score >= $threshold
MATCH (rt:ReasoningTrace)-[:HAS_STEP]->(rs)
WHERE ($success_only = false OR rt.success = true)
RETURN rs, score, rt.id AS trace_id, rt.task AS task, rt.outcome AS outcome, rt.success AS trace_success
ORDER BY score DESC
";

pub const GET_TRACE_WITH_STEPS: &str = "
MATCH (rt:ReasoningTrace {id: $id})
OPTIONAL MATCH (rt)-[:HAS_STEP]->(rs:ReasoningStep)
OPTIONAL MATCH (rs)-[:USES_TOOL]->(tc:ToolCall)
RETURN rt,
       collect(DISTINCT rs) AS steps,
       collect(DISTINCT {step_id: rs.id, tool_call: tc}) AS tool_calls
";

pub const LIST_TRACES: &str = "
MATCH (rt:ReasoningTrace)
WHERE ($session_id IS NULL OR rt.session_id = $session_id)
  AND ($success IS NULL OR rt.success = $success)
  AND ($since IS NULL OR rt.started_at >= datetime($since))
  AND ($until IS NULL OR rt.started_at <= datetime($until))
WITH rt
ORDER BY
    CASE WHEN $order_dir = 'desc' THEN
        CASE WHEN $order_by = 'started_at' THEN rt.started_at
             WHEN $order_by = 'completed_at' THEN rt.completed_at
        END
    END DESC,
    CASE WHEN $order_dir = 'asc' THEN
        CASE WHEN $order_by = 'started_at' THEN rt.started_at
             WHEN $order_by = 'completed_at' THEN rt.completed_at
        END
    END ASC
SKIP $offset LIMIT $limit
RETURN rt
";

// --- Cross-memory links ---

pub const LINK_CONVERSATION_TO_TRACE: &str = "
MATCH (c:Conversation {id: $conversation_id, owner_scope: $owner_scope})
MATCH (rt:ReasoningTrace {id: $trace_id})
WITH * WHERE (rt.user_identifier = $user_identifier OR (rt.user_identifier IS NULL AND $user_identifier IS NULL))
MERGE (c)-[:HAS_TRACE]->(rt)
RETURN c, rt
";

pub const LINK_TRACE_TO_MESSAGE: &str = "
MATCH (rt:ReasoningTrace {id: $trace_id})
MATCH (:Conversation {owner_scope: $owner_scope})-[:HAS_MESSAGE]->(m:Message {id: $message_id})
WITH * WHERE (rt.user_identifier = $user_identifier OR (rt.user_identifier IS NULL AND $user_identifier IS NULL))
MERGE (rt)-[:INITIATED_BY]->(m)
RETURN rt, m
";

pub const LINK_TOOL_CALL_TO_MESSAGE: &str = "
MATCH (rt:ReasoningTrace)-[:HAS_STEP]->(:ReasoningStep)-[:USES_TOOL]->(tc:ToolCall {id: $tool_call_id})
MATCH (:Conversation {owner_scope: $owner_scope})-[:HAS_MESSAGE]->(m:Message {id: $message_id})
WITH * WHERE (rt.user_identifier = $user_identifier OR (rt.user_identifier IS NULL AND $user_identifier IS NULL))
MERGE (tc)-[:TRIGGERED_BY]->(m)
RETURN tc, m
";

pub const RECORD_TOUCHED_EDGE_BY_ID: &str = "
MATCH (s:ReasoningStep {id: $step_id})
MATCH (e:Entity {id: $entity_id})
MERGE (s)-[r:TOUCHED]->(e)
ON CREATE SET r.recorded_at = datetime()
RETURN s, e
";

/// Upstream `RECORD_TOUCHED_EDGE_BY_NAME_TYPE`.
///
/// SOURCE-CONFLICT: upstream gives an entity created here the id
/// `name + ':' + type`, which breaks the UUID entity ids required by TCK
/// SPEC-1.2.2 and Muninn's `Entity` model; a fresh UUID (`$new_id`) is used
/// instead. The lookup-key `SET` items are upstream `entity_keys_set_clause()`.
pub const RECORD_TOUCHED_EDGE_BY_NAME_TYPE: &str = "
MATCH (s:ReasoningStep {id: $step_id})
MERGE (e:Entity {name: $name, type: $type})
ON CREATE SET e.id = coalesce(e.id, $new_id),
              e.created_at = datetime(),
              e.name_key = toLower(toString(e.name)),
    e.surface_keys = reduce(acc = '|', form IN [e.name, e.canonical_name] + coalesce(e.aliases, []) | CASE WHEN form IS NULL THEN acc ELSE acc + replace(toLower(toString(form)), '|', ' ') + '|' END)
MERGE (s)-[r:TOUCHED]->(e)
ON CREATE SET r.recorded_at = datetime()
RETURN s, e
";

/// SOURCE-CONFLICT: as [`RECORD_TOUCHED_EDGE_BY_NAME_TYPE`], upstream uses
/// the bare name as the new entity's id; a fresh UUID is used instead.
pub const RECORD_TOUCHED_EDGE_BY_NAME: &str = "
MATCH (s:ReasoningStep {id: $step_id})
MERGE (e:Entity {name: $name})
ON CREATE SET e.id = coalesce(e.id, $new_id),
              e.created_at = datetime(),
              e.name_key = toLower(toString(e.name)),
    e.surface_keys = reduce(acc = '|', form IN [e.name, e.canonical_name] + coalesce(e.aliases, []) | CASE WHEN form IS NULL THEN acc ELSE acc + replace(toLower(toString(form)), '|', ' ') + '|' END)
MERGE (s)-[r:TOUCHED]->(e)
ON CREATE SET r.recorded_at = datetime()
RETURN s, e
";

// --- Hosted (Platinum) conversation view ---

pub const GET_OPEN_CONVERSATION_TRACE: &str = "
MATCH (:Conversation {id: $conversation_id, owner_scope: $owner_scope})-[:HAS_TRACE]->(rt:ReasoningTrace)
WHERE rt.completed_at IS NULL
RETURN rt.id AS id
ORDER BY rt.started_at DESC
LIMIT 1
";

/// Return the conversation's newest open trace or create and link one. The
/// conversation node is write-locked first so concurrent callers agree on a
/// single trace.
pub const ENSURE_CONVERSATION_TRACE: &str = "
MATCH (c:Conversation {id: $conversation_id, owner_scope: $owner_scope})
SET c._muninn_lock = true
REMOVE c._muninn_lock
WITH c
OPTIONAL MATCH (c)-[:HAS_TRACE]->(open:ReasoningTrace)
WHERE open.completed_at IS NULL
WITH c, open
ORDER BY open.started_at DESC
LIMIT 1
FOREACH (_ IN CASE WHEN open IS NULL THEN [1] ELSE [] END |
    CREATE (rt:ReasoningTrace {
        id: $id,
        user_identifier: $user_identifier,
        session_id: c.session_id,
        task: $task,
        task_embedding: $task_embedding,
        outcome: null,
        success: null,
        started_at: datetime(),
        completed_at: null,
        metadata: $metadata
    })
    CREATE (c)-[:HAS_TRACE]->(rt)
)
RETURN coalesce(open.id, $id) AS id
";

pub const GET_CONVERSATION_STEPS: &str = "
MATCH (c:Conversation {id: $conversation_id, owner_scope: $owner_scope})
OPTIONAL MATCH (c)-[:HAS_TRACE]->(rt:ReasoningTrace)-[:HAS_STEP]->(rs:ReasoningStep)
OPTIONAL MATCH (rs)-[:USES_TOOL]->(tc:ToolCall)
WITH rt, rs, collect(tc) AS tool_calls
ORDER BY rt.started_at ASC, rs.step_number ASC
RETURN rt.id AS trace_id, rs, tool_calls
";

pub const GET_STEP_CONTEXT: &str = "
MATCH (rt:ReasoningTrace)-[:HAS_STEP]->(rs:ReasoningStep {id: $step_id})
MATCH (c:Conversation {owner_scope: $owner_scope})-[:HAS_TRACE]->(rt)
WITH rt, rs, head(collect(c.id)) AS conversation_id
OPTIONAL MATCH (rs)-[:USES_TOOL]->(tc:ToolCall)
WITH rt, rs, conversation_id, collect(tc) AS tool_calls
OPTIONAL MATCH (rs)-[:TOUCHED]->(e:Entity)
RETURN rt.id AS trace_id, rs, conversation_id, tool_calls, collect(DISTINCT e) AS entities
";

pub const GET_ENTITY_PROVENANCE: &str = "
MATCH (rs:ReasoningStep)-[:TOUCHED]->(:Entity {id: $entity_id})
MATCH (c:Conversation {owner_scope: $owner_scope})-[:HAS_TRACE]->(rt:ReasoningTrace)-[:HAS_STEP]->(rs)
RETURN DISTINCT c.id AS conversation_id, rt.id AS trace_id, rs
ORDER BY rs.timestamp ASC
";

pub const VALIDATE_MESSAGE_SCOPE: &str = "MATCH (:Conversation {owner_scope: $owner_scope})-[:HAS_MESSAGE]->(m:Message {id: $message_id}) RETURN m";

pub const CREATE_CONVERSATION_STEP: &str = "
MATCH (c:Conversation {id: $conversation_id, owner_scope: $owner_scope})-[:HAS_TRACE]->(rt:ReasoningTrace {id: $trace_id})
SET rt._muninn_lock = true
REMOVE rt._muninn_lock
WITH rt
OPTIONAL MATCH (rt)-[:HAS_STEP]->(existing:ReasoningStep)
WITH rt, coalesce(max(existing.step_number), 0) + 1 AS step_number
CREATE (rs:ReasoningStep {
    id: $id,
    step_number: step_number,
    thought: $thought,
    action: $action,
    observation: $observation,
    embedding: $embedding,
    timestamp: datetime(),
    metadata: $metadata
})
CREATE (rt)-[:HAS_STEP {order: step_number}]->(rs)
RETURN rs
";

/// Reject legacy conversations relevant to a hosted reasoning operation,
/// including legacy siblings that share the target conversation's session.
pub const LEGACY_CONFLICT: &str = "
MATCH (target:Conversation)
WHERE target.id = $guard_conversation OR
      EXISTS { MATCH (target)-[:HAS_MESSAGE]->(:Message {id: $guard_message}) } OR
      EXISTS { MATCH (target)-[:HAS_TRACE]->(:ReasoningTrace {id: $guard_trace}) } OR
      EXISTS { MATCH (target)-[:HAS_TRACE]->(:ReasoningTrace)-[:HAS_STEP]->(:ReasoningStep {id: $guard_step}) } OR
      EXISTS { MATCH (target)-[:HAS_TRACE]->(:ReasoningTrace)-[:HAS_STEP]->(:ReasoningStep)-[:USES_TOOL]->(:ToolCall {id: $guard_call}) } OR
      (target.owner_scope = $guard_owner AND EXISTS {
          MATCH (target)-[:HAS_TRACE]->(:ReasoningTrace)-[:HAS_STEP]->(:ReasoningStep)-[:TOUCHED]->(:Entity {id: $guard_entity}) })
WITH collect(target.id) AS ids, collect(target.session_id) AS sessions
MATCH (legacy:Conversation)
WHERE legacy.owner_scope IS NULL AND (legacy.id IN ids OR legacy.session_id IN sessions)
RETURN legacy LIMIT 1
";
