# Proposal: Muninn Extraction Pipeline, Semantic Resolution & Background Enrichment (`F-14`, `F-17`)

## Intent

Close the extraction, entity resolution, and background enrichment parity gaps (**F-14**, **F-17**) against the pinned Neo4j Agent Memory reference (`9d45af5529ec`) and establish verified differential evidence for **`GM-02`, `GM-03`, `GM-04`, `GM-06`, and `GM-07`**.

Currently, Muninn implements individual building blocks (explicit extraction, spaCy sidecar adapter, LLM extraction, Wikipedia/Diffbot enrichment, geocoding), but lacks combined multi-stage extraction pipeline parity (including GLiNER adapter support), complete semantic entity resolution parity, and bounded automatic background enrichment orchestration.

## Scope

### In Scope

- **Combined Extraction Pipeline & GLiNER Adapter (`F-14`, `GM-02`, `GM-04`):**
  - Implement a composable `ExtractionPipeline` in `muninn-application` supporting ordered stages (e.g., spaCy / GLiNER / LLM) with deterministic entity/relation merging, confidence thresholds, and provenance tracking (`BOLT-07-12..18`, `BOLT-07-25`).
  - Define an optional GLiNER extractor adapter behind a Cargo feature flag / sidecar port (mirroring the existing spaCy sidecar architecture so core builds stay lightweight and deterministic).
- **Semantic Entity Resolution Parity (`F-14`, `GM-03`):**
  - Complete hybrid/semantic entity resolution orchestration (exact → fuzzy → embedding similarity → intra-episode clustering) and deduplication review workflows (`BOLT-07-19..22`).
- **Automatic Background Enrichment (`F-17`, `GM-06`):**
  - Implement an opt-in bounded background enrichment worker reusing existing `EnrichmentService` and `GeocodingService` (`crates/muninn/application/src/enrichment.rs`), with bounded queue capacity, non-fatal provider error isolation, graceful shutdown draining, and zero impact on hot-path message latency.
- **Differential & Live Verification (`GM-02`, `GM-03`, `GM-04`, `GM-06`, `GM-07`):**
  - Execute and record commit-bound receipts using `dev/muninn/gm_h4_runner.py` and `gm05_runner.py` with deterministic mock/local providers and live Neo4j fixtures.

### Out of Scope

- H5 real-model retrieval acceptance protocol (owned by `.agents/sdd/changes/muninn-h5-acceptance-protocol`).
- `EvalMemory` API (owned by `muninn-eval-memory-api`).
- Paid external API calls in CI (CI tests use deterministic fake/local providers; live external enrichment remains opt-in).

## Approach

1. Wait for `SDD-1C` (`F-01`) to lock the atomic ingestion boundary in `crates/muninn/application/src/extraction.rs` before extending pipeline orchestration.
2. Build `ExtractionPipeline` and GLiNER sidecar port in `muninn-core` / `muninn-application` / `muninn-providers` via TDD.
3. Complete semantic entity resolution and deduplication review operations (`BOLT-07-19..22`) with live Neo4j tests.
4. Build bounded background enrichment worker following the same backpressure/shutdown discipline established in `SDD-2` (`BufferedWriter`).

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/muninn/core/src/extraction.rs`, `resolution.rs`, `enrichment.rs` | Modified | Pipeline config, GLiNER port, resolution/dedup review contracts. |
| `crates/muninn/application/src/extraction.rs`, `resolution.rs`, `enrichment.rs` | Modified | Composable `ExtractionPipeline`, semantic resolver, bounded background enrichment worker. |
| `crates/muninn/providers/src/` | Modified | Optional feature-gated GLiNER sidecar adapter. |
| `crates/muninn/neo4j/tests/` & `dev/muninn/` | Modified | Live Neo4j tests and `GM-02/03/04/06/07` differential verification. |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| GLiNER dependencies bloat default Cargo builds or break cross-platform CI. | High (if unconditional) | Gate behind optional Cargo feature / sidecar protocol matching existing spaCy sidecar design. |
| Background enrichment worker leaks tasks or stalls shutdown. | Medium | Bounded channel + explicit `stop()` drain + unit concurrency/cancellation tests. |
| Contention on `application/src/extraction.rs` with `SDD-1C`. | High (if parallel) | Sequence `extraction.rs` implementation strictly after `SDD-1C` lands. |

## Rollback Plan

Feature-gated and opt-in; revert via `git revert` without breaking existing explicit extraction, synchronous enrichment, or TCK Bronze/Silver/Gold conformance.

## Dependencies

- Implementation of `extraction.rs` depends on `SDD-1C` (`F-01` atomic ingestion boundary).
- Background enrichment worker reuses concurrency/shutdown patterns from `SDD-2` (`BufferedWriter`).

## Success Criteria & Common Definition of Done

- [ ] Composable extraction pipeline (spaCy / optional GLiNER / LLM) and provenance tracking pass unit and live Neo4j tests.
- [ ] Semantic entity resolution and deduplication review workflows (`BOLT-07-19..22`) pass live Neo4j tests.
- [ ] Opt-in background enrichment worker enforces bounded concurrency, non-fatal error isolation, and graceful shutdown.
- [ ] Differential scenarios `GM-02`, `GM-03`, `GM-04`, `GM-06`, and `GM-07` execute and record commit-bound receipts.
- [ ] Contract compared with the pinned upstream implementation (`9d45af5529ec`).
- [ ] Approved deviations explicitly documented.
- [ ] Hexagonal architecture boundaries preserved.
- [ ] RED → GREEN → REFACTOR evidence recorded.
- [ ] Unit and contract tests passed.
- [ ] Mandatory isolated Neo4j integration tests passed (`MUNINN_REQUIRE_NEO4J=1`).
- [ ] Concurrency, cancellation and failure paths verified.
- [ ] Owner isolation and authorization verified.
- [ ] Differential acceptance scenarios executed.
- [ ] Bazel and Cargo build inventory synchronized.
- [ ] Documentation and evidence registry synchronized.
- [ ] Acceptance receipts bound to the candidate commit.
- [ ] No skipped or ignored scenario counted as passed.
