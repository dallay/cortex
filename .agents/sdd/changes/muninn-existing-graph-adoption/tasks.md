# Tasks: Muninn Existing Graph Adoption (`BOLT-08-06`, SDD-3)

- [x] **3.1:** Define `is_safe_cypher_identifier`, `AdoptGraphOptions`, `AdoptionLabelReport`, `AdoptionReport`, and `SchemaAdoptionPort` in `crates/muninn/core/src/ports.rs` with upfront Cypher-injection validation tests.
- [x] **3.2:** Implement four-stage `SchemaManager::adopt_existing_graph` (`Validate → Plan → Apply → Verify`) and `SchemaAdoptionPort` for `Neo4jClient` in `crates/muninn/neo4j/src/schema.rs` (pre-flight ID conflict detection, zero-mutation `dry_run = true`, single-txn non-destructive apply with `adopted_at`/`adopted_from_label` provenance, full rollback on error).
- [x] **3.3:** Expose `with_schema_adoption` (fail-closed on read-only clients) and `adopt_existing_graph` on `MemoryClient` in `crates/muninn/application/src/client.rs`.
- [x] **3.4:** Workspace Clippy (`-D warnings`), `muninn-core`/`muninn-application`/`muninn-neo4j` unit tests, and Bazel inventory check all PASS (live Neo4j `GM-10` pending Docker).
