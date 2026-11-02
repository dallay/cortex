//! Short-term memory Cypher, ported from upstream `graph/queries.py`
//! (neo4j-labs/agent-memory @ 9d45af55, Apache-2.0).

pub const CREATE_CONVERSATION: &str = "
CREATE (c:Conversation {
    id: $id,
    session_id: $session_id,
    title: $title,
    created_at: datetime(),
    updated_at: datetime()
})
RETURN c
";

pub const GET_CONVERSATION: &str = "
MATCH (c:Conversation {id: $id})
WHERE c.owner_scope = $owner_scope
RETURN c
";

pub const GET_CONVERSATION_BY_SESSION: &str = "
MATCH (c:Conversation {session_id: $session_id})
WHERE c.owner_scope = $owner_scope
RETURN c
ORDER BY c.created_at DESC
LIMIT 1
";

pub const LIST_ALL_CONVERSATIONS: &str = "
MATCH (c:Conversation)
WHERE c.owner_scope = $owner_scope
RETURN c
ORDER BY c.updated_at DESC
LIMIT $limit
";

pub const LINK_USER_TO_CONVERSATION: &str = "
MERGE (u:User {identifier: $user_identifier})
ON CREATE SET u.id = $user_identifier, u.created_at = datetime()
WITH u
MATCH (c:Conversation {id: $conversation_id})
WHERE c.owner_scope = $owner_scope
MERGE (u)-[:HAS_CONVERSATION]->(c)
SET c.user_identifier = $user_identifier
";

pub const CREATE_MESSAGE: &str = "
MATCH (c:Conversation {id: $conversation_id})
WHERE c.owner_scope = $owner_scope
SET c.updated_at = datetime()
WITH c
OPTIONAL MATCH (c)-[:HAS_MESSAGE]->(last:Message)
WHERE NOT (last)-[:NEXT_MESSAGE]->()
WITH c, last ORDER BY last.timestamp DESC LIMIT 1
CREATE (m:Message {
    id: $id,
    role: $role,
    content: $content,
    embedding: $embedding,
    timestamp: CASE WHEN last IS NOT NULL AND last.timestamp >= datetime()
                    THEN last.timestamp + duration({microseconds: 1})
                    ELSE datetime()
               END,
    metadata: $metadata
})
CREATE (c)-[:HAS_MESSAGE]->(m)
FOREACH (_ IN CASE WHEN last IS NOT NULL THEN [1] ELSE [] END |
    CREATE (last)-[:NEXT_MESSAGE]->(m)
)
FOREACH (_ IN CASE WHEN last IS NULL THEN [1] ELSE [] END |
    CREATE (c)-[:FIRST_MESSAGE]->(m)
)
RETURN m
";

/// Upstream `CREATE_MESSAGES_BATCH`, returning the created nodes in input
/// order instead of only their count.
pub const CREATE_MESSAGES_BATCH: &str = "
MATCH (c:Conversation {id: $conversation_id})
WHERE c.owner_scope = $owner_scope
SET c.updated_at = datetime()
WITH c
OPTIONAL MATCH (c)-[:HAS_MESSAGE]->(last:Message)
WHERE NOT (last)-[:NEXT_MESSAGE]->()
WITH c, last ORDER BY last.timestamp DESC LIMIT 1
WITH c, last,
     CASE WHEN last IS NOT NULL AND last.timestamp >= datetime()
          THEN last.timestamp + duration({microseconds: 1})
          ELSE datetime()
     END AS base
UNWIND range(0, size($messages) - 1) AS i
WITH c, last, base, i, $messages[i] AS msg
CREATE (m:Message {
    id: msg.id,
    role: msg.role,
    content: msg.content,
    embedding: msg.embedding,
    timestamp: CASE WHEN msg.timestamp IS NOT NULL
                    THEN datetime(msg.timestamp)
                    ELSE base + duration({microseconds: i})
               END,
    metadata: msg.metadata
})
CREATE (c)-[:HAS_MESSAGE]->(m)
WITH c, last, i, m ORDER BY i
WITH c, last, collect(m) AS messages
FOREACH (first IN CASE WHEN last IS NULL THEN [head(messages)] ELSE [] END |
    CREATE (c)-[:FIRST_MESSAGE]->(first)
)
WITH messages, CASE WHEN last IS NULL THEN messages ELSE [last] + messages END AS chain
FOREACH (j IN range(0, size(chain) - 2) |
    FOREACH (prev IN [chain[j]] |
        FOREACH (next IN [chain[j + 1]] |
            CREATE (prev)-[:NEXT_MESSAGE]->(next)
        )
    )
)
WITH messages
UNWIND messages AS m
RETURN m
";

pub const GET_CONVERSATION_MESSAGES: &str = "
MATCH (c:Conversation {id: $conversation_id})-[:HAS_MESSAGE]->(m:Message)
RETURN m
ORDER BY m.timestamp ASC, m.id ASC
LIMIT $limit
";

/// Exact scoped search: candidate selection must not take a global ANN top-k
/// before applying the namespace. Native scoped ANN acceptance is a separate gate.
pub const SEARCH_MESSAGES_BY_EMBEDDING: &str = "
MATCH (c:Conversation)-[:HAS_MESSAGE]->(m:Message)
WHERE c.owner_scope = $owner_scope
  AND m.embedding IS NOT NULL AND size(m.embedding) = size($embedding)
WITH m, vector.similarity.cosine(m.embedding, $embedding) AS score
WHERE score >= $threshold
RETURN DISTINCT m, score
ORDER BY score DESC
LIMIT $limit
";

/// Session-scoped exact search.
///
/// Upstream accepts `session_id` but ignores it (SOURCE-CONFLICT with TCK
/// SPEC-2.3.2); Muninn scores the session's own
/// messages with the same cosine normalisation the vector index uses.
pub const SEARCH_SESSION_MESSAGES_BY_EMBEDDING: &str = "
MATCH (c:Conversation {session_id: $session_id})-[:HAS_MESSAGE]->(m:Message)
WHERE c.owner_scope = $owner_scope
  AND m.embedding IS NOT NULL AND size(m.embedding) = size($embedding)
WITH m, vector.similarity.cosine(m.embedding, $embedding) AS score
WHERE score >= $threshold
RETURN m, score
ORDER BY score DESC
LIMIT $limit
";

/// Upstream `SEARCH_MESSAGES_WITH_METADATA_TEMPLATE`.
pub const SEARCH_MESSAGES_WITH_METADATA_TEMPLATE: &str = "
MATCH (c:Conversation)-[:HAS_MESSAGE]->(m:Message)
WHERE c.owner_scope = $owner_scope
  AND m.embedding IS NOT NULL AND size(m.embedding) = size($embedding)
  AND m.metadata IS NOT NULL AND {metadata_clause}
WITH m, vector.similarity.cosine(m.embedding, $embedding) AS score
WHERE score >= $threshold
RETURN DISTINCT m, score
ORDER BY score DESC
LIMIT $limit
";

pub const SEARCH_SESSION_MESSAGES_WITH_METADATA_TEMPLATE: &str = "
MATCH (c:Conversation {session_id: $session_id})-[:HAS_MESSAGE]->(m:Message)
WHERE c.owner_scope = $owner_scope
  AND m.embedding IS NOT NULL AND size(m.embedding) = size($embedding)
  AND m.metadata IS NOT NULL AND {metadata_clause}
WITH m, vector.similarity.cosine(m.embedding, $embedding) AS score
WHERE score >= $threshold
RETURN m, score
ORDER BY score DESC
LIMIT $limit
";

/// Chain-repairing delete. Upstream `DELETE_MESSAGE` uses plain `DELETE`,
/// which fails while `HAS_MESSAGE`/`NEXT_MESSAGE` edges exist; the pinned TCK
/// reference adapter replaces it with this statement.
pub const DELETE_MESSAGE: &str = "
MATCH (owner:Conversation {owner_scope: $owner_scope})-[:HAS_MESSAGE]->(m:Message {id: $id})
OPTIONAL MATCH (prev)-[:NEXT_MESSAGE]->(m)
OPTIONAL MATCH (m)-[:NEXT_MESSAGE]->(next)
OPTIONAL MATCH (conv)-[:FIRST_MESSAGE]->(m)
FOREACH (_ IN CASE WHEN prev IS NOT NULL AND next IS NOT NULL THEN [1] ELSE [] END |
    CREATE (prev)-[:NEXT_MESSAGE]->(next)
)
FOREACH (_ IN CASE WHEN conv IS NOT NULL AND next IS NOT NULL THEN [1] ELSE [] END |
    CREATE (conv)-[:FIRST_MESSAGE]->(next)
)
DETACH DELETE m
RETURN true AS deleted
";

pub const LIST_SESSIONS: &str = "
MATCH (c:Conversation)
WHERE c.owner_scope = $owner_scope AND ($prefix IS NULL OR c.session_id STARTS WITH $prefix)
WITH c
OPTIONAL MATCH (c)-[:HAS_MESSAGE]->(m:Message)
WITH c,
     count(m) AS message_count,
     min(m.timestamp) AS first_msg_time,
     max(m.timestamp) AS last_msg_time,
     collect(m) AS messages
WITH c, message_count, first_msg_time, last_msg_time,
     head([msg IN messages WHERE msg.timestamp = first_msg_time | msg.content]) AS first_content,
     head([msg IN messages WHERE msg.timestamp = last_msg_time | msg.content]) AS last_content
WITH c.session_id AS session_id,
     c.title AS title,
     c.created_at AS created_at,
     c.updated_at AS updated_at,
     message_count,
     substring(first_content, 0, 100) AS first_message_preview,
     substring(last_content, 0, 100) AS last_message_preview
ORDER BY
    CASE WHEN $order_by = 'created_at' AND $order_dir = 'desc' THEN created_at END DESC,
    CASE WHEN $order_by = 'created_at' AND $order_dir = 'asc' THEN created_at END ASC,
    CASE WHEN $order_by = 'updated_at' AND $order_dir = 'desc' THEN updated_at END DESC,
    CASE WHEN $order_by = 'updated_at' AND $order_dir = 'asc' THEN updated_at END ASC,
    CASE WHEN $order_by = 'message_count' AND $order_dir = 'desc' THEN message_count END DESC,
    CASE WHEN $order_by = 'message_count' AND $order_dir = 'asc' THEN message_count END ASC
SKIP $offset LIMIT $limit
RETURN session_id, title, created_at, updated_at, message_count, first_message_preview, last_message_preview
";

pub const DELETE_SESSION_DATA: &str = "
MATCH (c:Conversation {session_id: $session_id})
WHERE c.owner_scope = $owner_scope
OPTIONAL MATCH (c)-[:HAS_MESSAGE]->(m:Message)
OPTIONAL MATCH (c)-[:HAS_TRACE]->(rt:ReasoningTrace)
OPTIONAL MATCH (rt)-[:HAS_STEP]->(rs:ReasoningStep)
OPTIONAL MATCH (rs)-[:USES_TOOL]->(tc:ToolCall)
DETACH DELETE c, m, rt, rs, tc
";

pub const DELETE_CONVERSATION: &str = "
MATCH (c:Conversation {id: $id})
WHERE c.owner_scope = $owner_scope
OPTIONAL MATCH (c)-[:HAS_MESSAGE]->(m:Message)
DETACH DELETE c, m
";

#[cfg(test)]
mod tests {
    use super::*;

    /// Splits a Cypher statement into its top-level clauses.
    ///
    /// A new clause starts at every newline that appears outside any bracket,
    /// so multi-line `FOREACH` blocks remain a single clause.
    fn top_level_clauses(query: &str) -> Vec<String> {
        let mut clauses = Vec::new();
        let mut current = String::new();
        let mut depth: i32 = 0;
        for ch in query.chars() {
            match ch {
                '(' | '[' | '{' => {
                    depth += 1;
                    current.push(ch);
                }
                ')' | ']' | '}' => {
                    depth -= 1;
                    current.push(ch);
                }
                '\n' if depth == 0 => clauses.push(std::mem::take(&mut current)),
                _ => current.push(ch),
            }
        }
        clauses.push(current);
        clauses
            .into_iter()
            .map(|clause| clause.trim().to_string())
            .filter(|clause| !clause.is_empty())
            .collect()
    }

    /// Returns the leading keyword of a clause, such as `MATCH` or `UNWIND`.
    fn clause_keyword(clause: &str) -> &str {
        clause
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim_matches(|c: char| !c.is_ascii_alphabetic())
    }

    #[test]
    fn create_messages_batch_separates_foreach_from_unwind() {
        let clauses = top_level_clauses(CREATE_MESSAGES_BATCH);
        assert!(
            clauses
                .iter()
                .any(|clause| clause_keyword(clause) == "FOREACH"),
            "expected CREATE_MESSAGES_BATCH to keep its FOREACH blocks"
        );
        for pair in clauses.windows(2) {
            assert!(
                !(clause_keyword(&pair[0]) == "FOREACH" && clause_keyword(&pair[1]) == "UNWIND"),
                "Neo4j requires a WITH clause between FOREACH and UNWIND, but found: {:?}",
                pair
            );
        }
    }

    #[test]
    fn create_messages_batch_returns_created_nodes_in_input_order() {
        assert!(CREATE_MESSAGES_BATCH.contains("UNWIND messages AS m"));
        assert!(CREATE_MESSAGES_BATCH.trim_end().ends_with("RETURN m"));
    }
}

#[cfg(test)]
mod owner_scope_tests {
    use super::*;

    #[test]
    fn all_search_variants_filter_conversation_owner_before_limit() {
        for query in [
            SEARCH_MESSAGES_BY_EMBEDDING,
            SEARCH_SESSION_MESSAGES_BY_EMBEDDING,
            SEARCH_MESSAGES_WITH_METADATA_TEMPLATE,
            SEARCH_SESSION_MESSAGES_WITH_METADATA_TEMPLATE,
        ] {
            let predicate = "c.owner_scope = $owner_scope";
            let owner = query
                .find(predicate)
                .expect("search must filter by conversation owner");
            assert!(owner < query.find("LIMIT $limit").unwrap());
            assert!(query.contains("(c:Conversation"));
            assert!(query.contains("-[:HAS_MESSAGE]->(m:Message)"));
        }
    }
}

/// Resolve identity without adopting legacy rows or transferring ownership.
/// Composite uniqueness and UUID uniqueness are required by schema setup.
pub const ENSURE_SCOPED_CONVERSATION: &str = "
OPTIONAL MATCH (legacy:Conversation {session_id: $session_id})
WHERE legacy.owner_scope IS NULL
WITH count(legacy) AS legacy_count
WHERE legacy_count = 0
OPTIONAL MATCH (by_id:Conversation {id: $id})
WITH by_id
WHERE by_id IS NULL OR
      (by_id.owner_scope = $owner_scope AND by_id.session_id = $session_id)
OPTIONAL MATCH (existing:Conversation {owner_scope: $owner_scope, session_id: $session_id})
WITH existing
WHERE NOT $explicit_id OR existing IS NULL OR existing.id = $id
MERGE (c:Conversation {owner_scope: $owner_scope, session_id: $session_id})
ON CREATE SET c.id = $id, c.user_identifier = $user_identifier,
              c.created_at = datetime(), c.updated_at = datetime()
WITH c
WHERE NOT $explicit_id OR c.id = $id
FOREACH (_ IN CASE WHEN $user_identifier IS NULL THEN [] ELSE [1] END |
    MERGE (u:User {identifier: $user_identifier})
    ON CREATE SET u.id = $user_identifier, u.created_at = datetime()
    MERGE (u)-[:HAS_CONVERSATION]->(c)
)
RETURN c
";

pub const GET_CONVERSATION_BY_ID_AND_SESSION: &str = "
MATCH (c:Conversation {id: $id, session_id: $session_id, owner_scope: $owner_scope})
RETURN c
";

/// Scoped replay pre-check. The write path still repeats this lookup under
/// the conversation lock to make the final insert atomic.
pub const FIND_MESSAGE_BY_OPERATION_KEY: &str = "
MATCH (c:Conversation {id: $conversation_id})
WHERE c.owner_scope = $owner_scope
MATCH (c)-[:HAS_MESSAGE]->(m:Message {operation_key: $operation_key})
RETURN m
";

/// Serialize keyed lookup and chain append using the conversation write lock.
pub const CREATE_KEYED_MESSAGE: &str = "
MATCH (c:Conversation {id: $conversation_id})
WHERE c.owner_scope = $owner_scope
SET c.updated_at = c.updated_at
WITH c
OPTIONAL MATCH (c)-[:HAS_MESSAGE]->(existing:Message {operation_key: $operation_key})
WITH c, existing
WHERE existing IS NULL OR existing.operation_payload = $operation_payload
CALL {
WITH c, existing
WITH existing WHERE existing IS NOT NULL
RETURN existing AS m
UNION
WITH c, existing
WITH c WHERE existing IS NULL
OPTIONAL MATCH (c)-[:HAS_MESSAGE]->(last:Message)
WHERE NOT (last)-[:NEXT_MESSAGE]->()
WITH c, last ORDER BY last.timestamp DESC LIMIT 1
CREATE (m:Message {
    id: $id,
    role: $role,
    content: $content,
    embedding: $embedding,
    timestamp: CASE WHEN last IS NOT NULL AND last.timestamp >= datetime()
                    THEN last.timestamp + duration({microseconds: 1})
                    ELSE datetime()
               END,
    metadata: $metadata
})
SET c.updated_at = datetime()
SET m.operation_key = $operation_key, m.operation_payload = $operation_payload,
    m.operation_conversation_id = $conversation_id
CREATE (c)-[:HAS_MESSAGE]->(m)
FOREACH (_ IN CASE WHEN last IS NOT NULL THEN [1] ELSE [] END |
    CREATE (last)-[:NEXT_MESSAGE]->(m)
)
FOREACH (_ IN CASE WHEN last IS NULL THEN [1] ELSE [] END |
    CREATE (c)-[:FIRST_MESSAGE]->(m)
)
RETURN m
}
RETURN m
";
