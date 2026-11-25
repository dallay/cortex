# Proposal: Muninn Spatial Retrieval & Location Projection (`F-16`, `BOLT-08-03/13/14`)

## Intent

Implement geographic retrieval and location projection capabilities (`BOLT-08-03`, `BOLT-08-13`, `BOLT-08-14`) matching the pinned Neo4j Agent Memory contract (`9d45af5529ec`). Muninn already creates the `entity_location_idx` point index (`crates/muninn/neo4j/src/schema.rs:75`) and supports geocoding coordinate writes (`crates/muninn/application/src/enrichment.rs`), but lacks the query-side spatial APIs (`search_locations_near`, `search_locations_in_bounding_box`, `get_location_coordinates`, `get_locations`).

## Scope

### In Scope

- **Core Spatial Types & Port (`muninn-core`):**
  - Define `BoundingBox`, `LocationSearchResult` (entity + distance), `SessionLocationProjection`, and coordinate validation (latitude `[-90.0, 90.0]`, longitude `[-180.0, 180.0]`, non-negative radius, valid min/max bounds).
  - Extend `LongTermStore` / spatial port with:
    - `search_locations_near(lat, lon, radius_km, limit, scope)` (`BOLT-08-03`)
    - `search_locations_in_bounding_box(min_lat, min_lon, max_lat, max_lon, limit, scope)` (`BOLT-08-03`)
    - `get_location_coordinates(entity_id, scope)` (`BOLT-08-14`)
    - `get_locations(session_id, limit, scope)` projecting locations and linked conversations via `EXTRACTED_FROM` (`BOLT-08-13`)
- **Neo4j Adapter (`muninn-neo4j`):**
  - Implement Cypher queries using Neo4j `point({latitude, longitude})`, `point.distance(...)`, and `point.withinBBox(...)` backed by `entity_location_idx`, respecting scope classification (`SDD-1D`) and deterministic distance ordering.
- **Application & Bridge Surface (`muninn-application`, `muninn-bridge`):**
  - Expose all four methods on `LongTermMemory` / `MemoryClient` and wire them into `muninn-bridge`.
- **Verification (TDD + Live Neo4j `MUNINN_REQUIRE_NEO4J=1`):**
  - Unit tests for coordinate/radius/bounding-box validation (NaN, out-of-range lat/lon, inverted bbox, negative radius).
  - Live Neo4j tests (`crates/muninn/neo4j/tests/spatial_retrieval.rs`) verifying interior, exact boundary, exterior points, ascending distance ordering, missing coordinates (`None`), and session projection via `EXTRACTED_FROM`.

### Out of Scope

- External geocoding provider integrations beyond existing `GeocodingService` (`BOLT-08-15`).
- Polygon / GeoJSON complex spatial indexing beyond radial and bounding-box queries.

## Approach

1. Define validated spatial value objects in `muninn-core` rejecting NaN/Inf and out-of-bounds coordinates fail-closed.
2. Implement parameterized Neo4j spatial Cypher queries in `muninn-neo4j` using `entity_location_idx`.
3. Expose via `LongTermMemory` / `MemoryClient` and `muninn-bridge`, and verify with deterministic fixtures against live Neo4j.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/muninn/core/src/model.rs` & `ports.rs` | Modified | Spatial query/result types, bounds validation, and store port methods. |
| `crates/muninn/neo4j/src/long_term.rs` & `queries/` | Modified | `point.distance`, `point.withinBBox`, coordinate lookup, and `EXTRACTED_FROM` projection. |
| `crates/muninn/application/src/long_term.rs` & `client.rs` | Modified | Public spatial retrieval and `get_locations` methods. |
| `crates/muninn/bridge/src/` & `neo4j/tests/spatial_retrieval.rs` | Modified / New | Bridge bindings and live Neo4j spatial tests. |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Neo4j `point.distance` uses meters while upstream API uses kilometers (unit mismatch). | Medium | Explicitly verify upstream units (`km` vs `m`) in `spec` and test exact boundary distances against live Neo4j. |
| Antimeridian (`±180°`) bounding boxes behave unexpectedly in `point.withinBBox`. | Low | Document and test bounding-box constraints explicitly in `spec`. |

## Rollback Plan

Purely additive query API; revert via `git revert` without schema or data impact.

## Dependencies

- Aligns with `SDD-1D` scope policy and `SDD-3` (adopted location entities become spatially searchable once geocoded).

## Success Criteria & Common Definition of Done

- [ ] `search_locations_near`, `search_locations_in_bounding_box`, `get_location_coordinates`, and `get_locations` pass unit and live Neo4j boundary/ordering tests.
- [ ] Invalid coordinates, NaN/Inf, negative radius, and inverted bounding boxes fail closed.
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
