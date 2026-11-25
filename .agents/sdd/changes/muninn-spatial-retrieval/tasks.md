# Tasks: Muninn Spatial Retrieval & Location Projection (`SDD-5C`, `F-16`, `BOLT-08-03/13/14`)

- [x] **5C.1:** Define `GeoPoint`, `BoundingBox`, `SpatialLocationHit`, `SessionLocation`, and `SpatialStore` port in `crates/muninn/core/src/ports.rs` with fail-closed bounds/NaN validation unit tests.
- [x] **5C.2:** Implement `Neo4jSpatialStore` in `crates/muninn/neo4j/src/spatial.rs` (`search_locations_near`, `search_locations_in_bounding_box`, `get_location_coordinates`, `get_locations` via `EXTRACTED_FROM`).
- [x] **5C.3:** Expose spatial methods on `MemoryClient` and wire `Neo4jSpatialStore` in `apps/muninn/src/main.rs`.
- [ ] **5C.4:** Run live Neo4j spatial boundary/ordering integration tests under `MUNINN_REQUIRE_NEO4J=1` when Docker is available.
