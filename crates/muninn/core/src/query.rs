//! Read-only Cypher access (`core/query.py`).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::{MemoryError, MemoryResult};

/// Upstream `_WRITE_PATTERNS`, matched on the upper-cased query.
const WRITE_KEYWORDS: &[&[&str]] = &[
    &["CREATE"],
    &["MERGE"],
    &["DELETE"],
    &["SET"],
    &["REMOVE"],
    &["DROP"],
    &["LOAD", "CSV"],
    &["FOREACH"],
    &["IN", "TRANSACTIONS"],
];

/// Procedures callable from read-only queries.
///
/// CORTEX hardening: upstream accepts every `CALL x.y(...)`, which lets
/// procedures such as `apoc.periodic.iterate` or `apoc.cypher.doIt` run
/// writes in their own transactions. Muninn only allows known read-only
/// procedure namespaces.
const READ_PROCEDURE_PREFIXES: &[&str] = &[
    "DB.INDEX.VECTOR.QUERYNODES",
    "DB.INDEX.VECTOR.QUERYRELATIONSHIPS",
    "DB.INDEX.FULLTEXT.QUERYNODES",
    "DB.INDEX.FULLTEXT.QUERYRELATIONSHIPS",
    "DB.LABELS",
    "DB.RELATIONSHIPTYPES",
    "DB.PROPERTYKEYS",
    "DB.SCHEMA.",
    "APOC.META.",
    "APOC.PATH.",
];

fn words(query: &str) -> Vec<String> {
    query
        .split(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '.' || ch == '{'))
        .filter(|word| !word.is_empty())
        .map(str::to_uppercase)
        .collect()
}

/// Whether `query` is acceptable as read-only Cypher.
///
/// Rejects upstream's write keywords (as whole words), `CALL {` subqueries
/// and any procedure outside [`READ_PROCEDURE_PREFIXES`].
pub fn is_read_only_query(query: &str) -> bool {
    let tokens = words(query);
    let has_sequence = |sequence: &[&str]| {
        tokens
            .windows(sequence.len())
            .any(|window| window.iter().zip(sequence).all(|(a, b)| a == b))
    };
    if WRITE_KEYWORDS.iter().any(|sequence| has_sequence(sequence)) {
        return false;
    }
    for (index, token) in tokens.iter().enumerate() {
        if token == "CALL" {
            let Some(next) = tokens.get(index + 1) else {
                return false;
            };
            if next.starts_with('{')
                || !READ_PROCEDURE_PREFIXES
                    .iter()
                    .any(|prefix| next.starts_with(prefix))
            {
                return false;
            }
        }
    }
    true
}

/// Columnar query result (TCK `TCKCypherResult`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CypherResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

/// Read-only Cypher execution (`CypherQueryProtocol`).
#[async_trait]
pub trait CypherQuery: Send + Sync {
    /// Execute a query that already passed [`is_read_only_query`].
    async fn run_read_only(
        &self,
        query: &str,
        params: &Map<String, Value>,
    ) -> MemoryResult<CypherResult>;

    /// Validate and execute a read-only query.
    async fn cypher(&self, query: &str, params: &Map<String, Value>) -> MemoryResult<CypherResult> {
        if !is_read_only_query(query) {
            return Err(MemoryError::invalid(
                "Only read-only Cypher queries are allowed. Detected write keywords \
                 (CREATE/MERGE/DELETE/SET/...) or a procedure outside the read-only allowlist. \
                 Use the appropriate memory-layer method for writes.",
            ));
        }
        self.run_read_only(query, params).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_reads_and_read_procedures() {
        assert!(is_read_only_query(
            "MATCH (n) RETURN count(n) AS total LIMIT 1"
        ));
        assert!(is_read_only_query(
            "CALL db.index.vector.queryNodes('message_embedding_idx', 5, $e) YIELD node RETURN node"
        ));
        assert!(is_read_only_query(
            "MATCH (n:Entity) WHERE n.name = 'Settlement' RETURN n"
        ));
    }

    #[test]
    fn rejects_upstream_write_patterns() {
        for query in [
            "CREATE (n:X)",
            "match (n) detach delete n",
            "MATCH (n) SET n.x = 1",
            "MATCH (n) REMOVE n.x",
            "LOAD CSV FROM 'file:///x' AS row RETURN row",
            "MATCH (n) FOREACH (x IN [1] | SET n.a = x)",
            "MATCH (n) CALL { WITH n DELETE n } IN TRANSACTIONS",
            "DROP INDEX foo",
            "MERGE (n:X {id: 1})",
        ] {
            assert!(!is_read_only_query(query), "{query}");
        }
    }

    #[test]
    fn rejects_procedures_that_can_write() {
        for query in [
            "CALL apoc.periodic.iterate('MATCH (n) RETURN n', 'DETACH DELETE n', {})",
            "CALL apoc.cypher.doIt('CREATE (n)', {})",
            "CALL dbms.security.createUser('x', 'y')",
            "CALL {MATCH (n) RETURN n}",
            "MATCH (n) RETURN n CALL",
        ] {
            assert!(!is_read_only_query(query), "{query}");
        }
    }
}
