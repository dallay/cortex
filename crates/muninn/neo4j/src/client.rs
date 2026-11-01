//! Thin Neo4j client used by every store.

use muninn_core::{MemoryError, MemoryResult};
use neo4rs::{BoltType, ConfigBuilder, Graph, Query, Row};

/// Connection settings (`config.settings.Neo4jConfig`).
#[derive(Debug, Clone)]
pub struct Neo4jConfig {
    pub uri: String,
    pub username: String,
    pub password: String,
    pub database: Option<String>,
    pub max_connections: usize,
}

impl Default for Neo4jConfig {
    fn default() -> Self {
        Self {
            uri: "bolt://localhost:7687".to_owned(),
            username: "neo4j".to_owned(),
            password: String::new(),
            database: None,
            max_connections: 16,
        }
    }
}

/// Query parameters as `(name, value)` pairs.
pub type Params = Vec<(&'static str, BoltType)>;

#[derive(Clone)]
pub struct Neo4jClient {
    graph: Graph,
}

impl Neo4jClient {
    pub async fn connect(config: &Neo4jConfig) -> MemoryResult<Self> {
        let mut builder = ConfigBuilder::default()
            .uri(config.uri.as_str())
            .user(config.username.as_str())
            .password(config.password.as_str())
            .max_connections(config.max_connections);
        if let Some(database) = &config.database {
            builder = builder.db(database.as_str());
        }
        let neo4j_config = builder
            .build()
            .map_err(|err| MemoryError::storage(format!("invalid Neo4j configuration: {err}")))?;
        let graph = Graph::connect(neo4j_config)
            .await
            .map_err(|err| MemoryError::storage(format!("cannot connect to Neo4j: {err}")))?;
        let client = Self { graph };
        client.execute("RETURN 1 AS ok", Vec::new()).await?;
        Ok(client)
    }

    /// Run a statement and collect every row.
    pub async fn execute(&self, cypher: &str, params: Params) -> MemoryResult<Vec<Row>> {
        let mut query = Query::new(cypher.to_owned());
        for (name, value) in params {
            query = query.param(name, value);
        }
        let mut stream = self
            .graph
            .execute(query)
            .await
            .map_err(|err| MemoryError::storage(err.to_string()))?;
        let mut rows = Vec::new();
        while let Some(row) = stream
            .next()
            .await
            .map_err(|err| MemoryError::storage(err.to_string()))?
        {
            rows.push(row);
        }
        Ok(rows)
    }

    /// Run a statement with dynamically named parameters.
    pub async fn execute_dynamic(
        &self,
        cypher: &str,
        params: Vec<(String, BoltType)>,
    ) -> MemoryResult<Vec<Row>> {
        let mut query = Query::new(cypher.to_owned());
        for (name, value) in params {
            query = query.param(&name, value);
        }
        let mut stream = self
            .graph
            .execute(query)
            .await
            .map_err(|err| MemoryError::storage(err.to_string()))?;
        let mut rows = Vec::new();
        while let Some(row) = stream
            .next()
            .await
            .map_err(|err| MemoryError::storage(err.to_string()))?
        {
            rows.push(row);
        }
        Ok(rows)
    }

    /// Run a read-only statement inside a transaction that is always rolled
    /// back, so a write smuggled into it never commits.
    pub async fn execute_read_only(
        &self,
        cypher: &str,
        params: Vec<(String, BoltType)>,
    ) -> MemoryResult<Vec<Row>> {
        let mut txn = self
            .graph
            .start_txn()
            .await
            .map_err(|err| MemoryError::storage(err.to_string()))?;
        let mut query = Query::new(cypher.to_owned());
        for (name, value) in params {
            query = query.param(&name, value);
        }
        let result = async {
            let mut stream = txn
                .execute(query)
                .await
                .map_err(|err| MemoryError::storage(err.to_string()))?;
            let mut rows = Vec::new();
            while let Some(row) = stream
                .next(txn.handle())
                .await
                .map_err(|err| MemoryError::storage(err.to_string()))?
            {
                rows.push(row);
            }
            Ok::<_, MemoryError>(rows)
        }
        .await;
        let _ = txn.rollback().await;
        result
    }
}
