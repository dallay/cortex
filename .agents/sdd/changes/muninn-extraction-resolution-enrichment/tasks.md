# Tasks: Muninn Extraction Pipeline, Semantic Resolution & Background Enrichment (`SDD-5D`, `F-14`, `F-17`)

- [ ] **5D.1:** Implement composable `ExtractionPipeline` and optional feature-gated GLiNER sidecar adapter (`GM-02`, `GM-04`) building on `SDD-1C` atomic ingestion boundary.
- [ ] **5D.2:** Complete semantic entity resolution and deduplication review workflows (`BOLT-07-19..22`, `GM-03`).
- [ ] **5D.3:** Implement opt-in bounded background enrichment worker reusing `EnrichmentService`/`GeocodingService` and `SDD-2` backpressure/shutdown patterns (`GM-06`).
- [ ] **5D.4:** Run differential scenarios `GM-02`, `GM-03`, `GM-04`, `GM-06`, and `GM-07` against live Neo4j and record commit-bound receipts.
