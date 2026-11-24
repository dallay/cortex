//! `MemoryClient`: the composition of the three memory tiers.

use std::sync::Arc;

use muninn_core::ports::{
    AdoptGraphOptions, AdoptionReport, BoundingBox, BufferedWriteError, ConsolidationStore,
    Embedder, GeoPoint, LongTermStore, ReasoningStore, SchemaAdoptionPort, SessionLocation,
    ShortTermStore, SpatialLocationHit, SpatialStore,
};
use muninn_core::query::CypherQuery;
use muninn_core::{MemoryError, MemoryResult};

use crate::buffered::BufferedWriter;
use crate::consolidation::ConsolidationMemory;
use crate::eval::EvalMemory;
use crate::local_operator::{LocalOperator, LocalOperatorView};
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
    consolidation: Option<ConsolidationMemory>,
    buffered: Option<Arc<BufferedWriter>>,
    schema_adoption: Option<Arc<dyn SchemaAdoptionPort>>,
    spatial: Option<Arc<dyn SpatialStore>>,
    cypher_query: Option<Arc<dyn CypherQuery>>,
    local_operator_owner: Option<String>,
    local_operator_writes: bool,
}

impl MemoryClient {
    pub fn new(ports: MemoryPorts, options: &MemoryOptions) -> muninn_core::MemoryResult<Self> {
        // Validación antes de construir cualquier tier o etapa de ingestión.
        LongTermMemory::new(
            ports.long_term.clone(),
            ports.embedder.clone(),
            options.multi_tenant,
        )?;
        Ok(Self::build(ports, options, true))
    }

    /// Conserva consultas con adaptadores que no implementan ingestión atómica.
    pub fn read_only(ports: MemoryPorts, options: &MemoryOptions) -> Self {
        Self::build(ports, options, false)
    }

    fn build(ports: MemoryPorts, options: &MemoryOptions, writes: bool) -> Self {
        let ingestion = crate::extraction::IngestionExtraction {
            long_term: LongTermMemory::read_only(
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
            long_term: LongTermMemory::read_only(
                ports.long_term,
                ports.embedder.clone(),
                options.multi_tenant,
            ),
            reasoning: ReasoningMemory::new(ports.reasoning, ports.embedder, options.multi_tenant),
            consolidation: None,
            buffered: None,
            schema_adoption: None,
            spatial: None,
            cypher_query: ports.cypher_query,
            local_operator_owner: None,
            local_operator_writes: writes,
        };
        let mut ingestion = ingestion;
        if writes {
            client.long_term.enable_preference_writes();
            ingestion.long_term.enable_preference_writes();
        }
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

    /// Enable the in-process operator for a fixed, authorized owner at the
    /// trusted composition root. This does not grant writes to a read-only client.
    pub fn with_local_operator(mut self, trusted_owner: &str) -> MemoryResult<Self> {
        if self.local_operator_owner.is_some() {
            return Err(MemoryError::invalid(
                "local operator owner is already bound",
            ));
        }
        if trusted_owner.trim().is_empty() {
            return Err(MemoryError::invalid(
                "local operator owner must not be blank",
            ));
        }
        self.local_operator_owner = Some(trusted_owner.to_owned());
        Ok(self)
    }

    /// Typed local operator access; disabled by default.
    pub fn graph(&self) -> MemoryResult<LocalOperator<'_>> {
        let owner = self
            .local_operator_owner
            .as_deref()
            .ok_or_else(|| MemoryError::NotSupported("local operator is disabled".into()))?;
        Ok(LocalOperator::new(self, owner, self.local_operator_writes))
    }

    /// Restricted view of the three local operator entry points.
    pub const fn local_operator(&self) -> LocalOperatorView<'_> {
        LocalOperatorView::new(self)
    }

    /// Wire a consolidation store into the client (`client.consolidation()`).
    pub fn with_consolidation(mut self, store: Arc<dyn ConsolidationStore>) -> Self {
        self.consolidation = Some(ConsolidationMemory::from_arc(store));
        self
    }

    /// Wire an opt-in buffered writer (`client.buffered()`). Rejected fail-closed on read-only clients.
    pub fn with_buffered_writer(mut self, writer: Arc<BufferedWriter>) -> MemoryResult<Self> {
        if !self.local_operator_writes {
            return Err(MemoryError::NotSupported(
                "buffered writes are disabled on read-only MemoryClient".into(),
            ));
        }
        self.buffered = Some(writer);
        Ok(self)
    }

    /// Wire schema adoption (`BOLT-08-06`). Rejected fail-closed on read-only clients.
    pub fn with_schema_adoption(mut self, port: Arc<dyn SchemaAdoptionPort>) -> MemoryResult<Self> {
        if !self.local_operator_writes {
            return Err(MemoryError::NotSupported(
                "schema adoption is disabled on read-only MemoryClient".into(),
            ));
        }
        self.schema_adoption = Some(port);
        Ok(self)
    }

    /// Non-destructively adopt an existing Neo4j graph (`client.schema.adopt_existing_graph`).
    pub async fn adopt_existing_graph(
        &self,
        options: &AdoptGraphOptions,
    ) -> MemoryResult<AdoptionReport> {
        options.validate()?;
        let port = self.schema_adoption.as_ref().ok_or_else(|| {
            MemoryError::NotSupported("existing graph adoption is unavailable".into())
        })?;
        port.adopt_existing_graph(options).await
    }

    /// Wire a spatial retrieval store (`BOLT-08-03`, `BOLT-08-13`, `BOLT-08-14`).
    pub fn with_spatial_store(mut self, store: Arc<dyn SpatialStore>) -> Self {
        self.spatial = Some(store);
        self
    }

    /// Product evaluation API (`client.eval`, `BOLT-08-07`).
    pub fn eval(&self) -> EvalMemory<'_> {
        let extractor = self
            .short_term
            .ingestion
            .as_ref()
            .and_then(|i| i.extractor.clone());
        EvalMemory::new(&self.long_term, extractor)
    }

    /// Radial spatial entity search (`client.long_term.search_locations_near`, `BOLT-08-03`).
    pub async fn search_locations_near(
        &self,
        center: GeoPoint,
        radius_km: f64,
        limit: i64,
    ) -> MemoryResult<Vec<SpatialLocationHit>> {
        let store = self.spatial.as_ref().ok_or_else(|| {
            MemoryError::NotSupported("spatial retrieval store is unavailable".into())
        })?;
        store.search_locations_near(center, radius_km, limit).await
    }

    /// Bounding-box spatial entity search (`client.long_term.search_locations_in_bounding_box`, `BOLT-08-03`).
    pub async fn search_locations_in_bounding_box(
        &self,
        bbox: BoundingBox,
        limit: i64,
    ) -> MemoryResult<Vec<SpatialLocationHit>> {
        let store = self.spatial.as_ref().ok_or_else(|| {
            MemoryError::NotSupported("spatial retrieval store is unavailable".into())
        })?;
        store.search_locations_in_bounding_box(bbox, limit).await
    }

    /// Read coordinates for a location entity (`client.long_term.get_location_coordinates`, `BOLT-08-14`).
    pub async fn get_location_coordinates(
        &self,
        entity_id: uuid::Uuid,
    ) -> MemoryResult<Option<GeoPoint>> {
        let store = self.spatial.as_ref().ok_or_else(|| {
            MemoryError::NotSupported("spatial retrieval store is unavailable".into())
        })?;
        store.get_location_coordinates(entity_id).await
    }

    /// Project location entities and linked conversations via `EXTRACTED_FROM` (`client.get_locations`, `BOLT-08-13`).
    pub async fn get_locations(
        &self,
        session_id: Option<&str>,
        limit: i64,
    ) -> MemoryResult<Vec<SessionLocation>> {
        let store = self.spatial.as_ref().ok_or_else(|| {
            MemoryError::NotSupported("spatial retrieval store is unavailable".into())
        })?;
        store.get_locations(session_id, limit).await
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

    pub const fn consolidation(&self) -> Option<&ConsolidationMemory> {
        self.consolidation.as_ref()
    }

    /// Accessor to the buffered writer (`client.buffered`); `None` when not initialized.
    pub fn buffered(&self) -> Option<&BufferedWriter> {
        self.buffered.as_deref()
    }

    /// Flush queued buffered writes (`client.flush`); no-op if writer is uninitialized.
    pub async fn flush(&self) -> MemoryResult<()> {
        match &self.buffered {
            Some(writer) => writer.flush().await,
            None => Ok(()),
        }
    }

    /// Alias of [`Self::flush`] (`client.wait_for_pending`).
    pub async fn wait_for_pending(&self) -> MemoryResult<()> {
        self.flush().await
    }

    /// Background write errors (`client.write_errors`); empty when writer is uninitialized.
    pub fn write_errors(&self) -> Vec<BufferedWriteError> {
        self.buffered
            .as_deref()
            .map(BufferedWriter::errors)
            .unwrap_or_default()
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
