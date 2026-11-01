use async_trait::async_trait;
use muninn_core::ports::GraphMaintenance;
use muninn_core::MemoryResult;

use crate::client::Neo4jClient;

#[derive(Clone)]
pub struct Neo4jMaintenance {
    client: Neo4jClient,
}

impl Neo4jMaintenance {
    pub const fn new(client: Neo4jClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl GraphMaintenance for Neo4jMaintenance {
    async fn clear_all_data(&self) -> MemoryResult<()> {
        self.client
            .execute("MATCH (n) DETACH DELETE n", Vec::new())
            .await
            .map(|_| ())
    }
}
