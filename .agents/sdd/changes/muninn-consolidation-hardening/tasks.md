# Tasks: Muninn Consolidation Hardening & Production Wiring (`SDD-4`)

- [x] **4.1 (F-05):** Remove fallback `record_run` from `ConsolidationMemory` and add `ConsolidationReport::validate_invariants()` in `crates/muninn/core/src/ports.rs` enforcing single transactional audit ownership.
- [x] **4.2 (F-06):** Align defaults with pinned upstream (`dedupe 0.95 / 10_000`, `min_steps 20 / 1_000`, `supersede 0.92`, mandatory explicit `ttl_days`) and add upfront `ConsolidationOptions::validate()` before `run_all_with_options()` executes any job.
- [x] **4.3 (F-08):** Replace order-sensitive `WHERE old_owners = new_owners` with set equality in `FIND_SUPERSEDED_PREFERENCES` (`crates/muninn/neo4j/src/queries/consolidation.rs`) and update `consolidation_queries.rs`.
- [x] **4.4 (F-09):** Add `detect_superseded_preferences_for_owner` across `ConsolidationStore`, `ConsolidationMemory`, and `Neo4jConsolidationStore` with `$user_identifier` Cypher anchoring and blank-owner rejection.
- [x] **4.5 (F-07):** Wire `Neo4jConsolidationStore` into `MemoryClient::with_consolidation(...)` in `apps/muninn/src/main.rs`.
- [x] **4.6:** Workspace Clippy (`-D warnings`), `consolidation::tests` (4/4 PASS), `consolidation_queries` (3/3 PASS), and Bazel inventory check all PASS (live Neo4j `GM09-C11` pending Docker).
