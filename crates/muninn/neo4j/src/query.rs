//! Read-only Cypher console (`core/query.py` `BoltCypherQuery`).

use std::collections::HashMap;

use async_trait::async_trait;
use muninn_core::query::{CypherQuery, CypherResult};
use muninn_core::MemoryResult;
use neo4rs::BoltType;
use serde_json::{Map, Value};

use crate::bolt::{from_json, to_json};
use crate::client::Neo4jClient;

#[derive(Clone)]
pub struct Neo4jCypherQuery {
    client: Neo4jClient,
}

impl Neo4jCypherQuery {
    pub const fn new(client: Neo4jClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl CypherQuery for Neo4jCypherQuery {
    /// Runs inside a transaction that is always rolled back, so nothing a
    /// query might write is ever committed.
    async fn run_read_only(
        &self,
        query: &str,
        params: &Map<String, Value>,
    ) -> MemoryResult<CypherResult> {
        let params = params
            .iter()
            .map(|(key, value)| (key.clone(), from_json(value)))
            .collect();
        let rows = self.client.execute_read_only(query, params).await?;
        let maps: Vec<HashMap<String, BoltType>> = rows
            .iter()
            .filter_map(|row| row.to_strict::<HashMap<String, BoltType>>().ok())
            .collect();
        let Some(first) = maps.first() else {
            return Ok(CypherResult::default());
        };
        let columns = column_order(query, first.keys().map(String::as_str).collect());
        let rows = maps
            .iter()
            .map(|map| {
                columns
                    .iter()
                    .map(|column| map.get(column).map_or(Value::Null, to_json))
                    .collect()
            })
            .collect();
        Ok(CypherResult { columns, rows })
    }
}

/// Order result columns as written in the final `RETURN` clause.
///
/// The driver hands rows back as maps, so the projection order is recovered
/// from the query text; when that fails the columns are sorted by name.
fn column_order(query: &str, mut keys: Vec<&str>) -> Vec<String> {
    keys.sort_unstable();
    let upper = query.to_uppercase();
    let Some(start) = upper.rfind("RETURN") else {
        return keys.into_iter().map(str::to_owned).collect();
    };
    let mut projection = &query[start + "RETURN".len()..];
    for clause in [" ORDER BY ", " SKIP ", " LIMIT "] {
        if let Some(end) = projection.to_uppercase().find(clause) {
            projection = &projection[..end];
        }
    }
    let mut ordered = Vec::new();
    let mut depth = 0_i32;
    let mut item = String::new();
    for ch in projection.chars().chain(std::iter::once(',')) {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
        if ch == ',' && depth == 0 {
            let text = item.trim();
            let name = text
                .to_uppercase()
                .rfind(" AS ")
                .map_or(text, |pos| text[pos + 4..].trim())
                .trim_matches('`');
            ordered.push(name.to_owned());
            item.clear();
        } else {
            item.push(ch);
        }
    }
    let mut sorted_ordered = ordered.clone();
    sorted_ordered.sort_unstable();
    if sorted_ordered == keys {
        ordered
    } else {
        keys.into_iter().map(str::to_owned).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_return_projection_order() {
        assert_eq!(
            column_order(
                "MATCH (n) RETURN n.name AS zeta, count(n) AS alpha ORDER BY alpha LIMIT 1",
                vec!["alpha", "zeta"]
            ),
            vec!["zeta", "alpha"]
        );
        assert_eq!(
            column_order(
                "MATCH (n) RETURN n, labels(n)[0]",
                vec!["n", "labels(n)[0]"]
            ),
            vec!["n", "labels(n)[0]"]
        );
    }

    #[test]
    fn falls_back_to_sorted_names() {
        assert_eq!(
            column_order("MATCH (n) RETURN *", vec!["b", "a"]),
            vec!["a", "b"]
        );
    }
}
