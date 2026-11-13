//! `MemoryClient`: the composition of the three memory tiers.

use std::sync::Arc;

use muninn_core::ports::{Embedder, LongTermStore, ReasoningStore, ShortTermStore};
use muninn_core::query::CypherQuery;

use crate::long_term::LongTermMemory;
use crate::reasoning::ReasoningMemory;
use crate::short_term::ShortTermMemory;

/// Feature switches shared by the memory tiers (`MemorySettings.memory`).
#[derive(Debug, Clone, Default)]
pub struct MemoryOptions {
    /// Require `user_identifier` on writes.
    pub multi_tenant: bool,
}

/// Ports wired by the composition root.
pub struct MemoryPorts {
    pub short_term: Arc<dyn ShortTermStore>,
    pub long_term: Arc<dyn LongTermStore>,
    pub reasoning: Arc<dyn ReasoningStore>,
    pub embedder: Option<Arc<dyn Embedder>>,
    pub cypher_query: Option<Arc<dyn CypherQuery>>,
}

pub struct MemoryClient {
    short_term: ShortTermMemory,
    long_term: LongTermMemory,
    reasoning: ReasoningMemory,
    cypher_query: Option<Arc<dyn CypherQuery>>,
}

impl MemoryClient {
    pub fn new(ports: MemoryPorts, options: &MemoryOptions) -> Self {
        let ingestion = crate::extraction::IngestionExtraction {
            long_term: LongTermMemory::new(
                ports.long_term.clone(),
                ports.embedder.clone(),
                options.multi_tenant,
            ),
            extractor: None,
        };
        let mut client = Self {
            short_term: ShortTermMemory::new(
                ports.short_term,
                ports.embedder.clone(),
                options.multi_tenant,
            ),
            long_term: LongTermMemory::new(
                ports.long_term,
                ports.embedder.clone(),
                options.multi_tenant,
            ),
            reasoning: ReasoningMemory::new(ports.reasoning, ports.embedder, options.multi_tenant),
            cypher_query: ports.cypher_query,
        };
        client.short_term.ingestion = Some(ingestion);
        client
    }

    /// Configure the trusted extraction port without changing legacy defaults.
    pub fn with_extractor(
        mut self,
        extractor: Arc<dyn muninn_core::extraction::EntityExtractor>,
    ) -> Self {
        if let Some(ingestion) = &mut self.short_term.ingestion {
            ingestion.extractor = Some(extractor);
        }
        self
    }

    /// Opt in to entity enrichment and geocoding controls. Both stay unreachable
    /// until a caller wires the services here.
    pub fn with_enrichment(mut self, service: Arc<crate::enrichment::EnrichmentService>) -> Self {
        self.long_term = self.long_term.clone().with_enrichment(service);
        self
    }

    pub fn with_geocoding(mut self, service: Arc<crate::enrichment::GeocodingService>) -> Self {
        self.long_term = self.long_term.clone().with_geocoding(service);
        self
    }

    pub const fn short_term(&self) -> &ShortTermMemory {
        &self.short_term
    }

    pub const fn long_term(&self) -> &LongTermMemory {
        &self.long_term
    }

    pub const fn reasoning(&self) -> &ReasoningMemory {
        &self.reasoning
    }

    pub async fn cypher(
        &self,
        query: &str,
        params: &serde_json::Map<String, serde_json::Value>,
    ) -> muninn_core::MemoryResult<muninn_core::query::CypherResult> {
        let port = self.cypher_query.as_ref().ok_or_else(|| {
            muninn_core::MemoryError::NotSupported(
                "read-only Cypher queries are unavailable".into(),
            )
        })?;
        port.cypher(query, params).await
    }
}
