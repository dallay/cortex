# Proposal: Muninn Consolidation Hardening & Production Wiring (`BOLT-08-05/08/09/10/11`)

## Intent

Close all five confirmed consolidation contract, transactional-ownership, and wiring defects (**F-05, F-06, F-07, F-08, F-09**) from Revision 4 of the Parity Audit so Muninn's persistent consolidation engine achieves exact behavioral parity with Neo4j Agent Memory (`9d45af5529ec`, `memory/consolidation.py`), enforces single transactional audit ownership, and is wired into the production composition root.

## Scope

### In Scope

- **F-05 (Eliminate Dual Audit Ownership — Single Transactional Responsibility):**
  - **Remove the orchestrator fallback audit write (`record_run`) from `ConsolidationMemory` completely.** Do *not* keep a secondary write path in `muninn-application` that tries to patch a missing `run_id` outside the store transaction.
  - Enforce the following port invariants on `ConsolidationStore` and validate them in `ConsolidationMemory` via `report.validate_invariants()?`:

| Execution Outcome | Mandatory Port Invariant |
|-------------------|--------------------------|
| `dry_run = true` | `mutations_applied == 0` AND `run_id.is_none()` |
| `dry_run = false`, `mutations_applied == 0` | `run_id.is_none()` (zero `:ConsolidationRun` nodes written) |
| `dry_run = false`, `mutations_applied > 0` | `run_id.is_some()`, persisted atomically in the **same** Neo4j transaction as the mutations |
| Audit persistence failure | Full transactional rollback of all mutations |

- **F-06 (Upstream Defaults & Upfront `run_all()` Config Validation — `GM09-C12`):**
  - Align defaults with pinned upstream `memory/consolidation.py`:
    - `DEFAULT_DEDUPE_THRESHOLD = 0.95`, `DEFAULT_DEDUPE_LIMIT = 10_000`
    - `DEFAULT_MIN_TRACE_STEPS = 20`, `DEFAULT_TRACE_LIMIT = 1_000`
    - `DEFAULT_SUPERSEDE_THRESHOLD = 0.92`
    - `archive_expired_conversations`: require explicit `ttl_days` (or configured `settings.memory.conversation_ttl_days`), returning `MemoryError::invalid` when omitted rather than silently applying 90 days.
  - **Upfront Configuration Validation for `run_all()`:** Validate the entire `ConsolidationOptions` (including required `ttl_days` when archival is enabled, positive limits, valid thresholds `[0.0, 1.0]`, and non-blank `user_identifier` if provided) **before executing the first job**, preventing `run_all()` from applying three mutations and then failing on a missing TTL.
- **F-08 (Order-Insensitive Owner-Set Comparison):**
  - Replace `WHERE old_owners = new_owners` in `FIND_SUPERSEDED_PREFERENCES` (`crates/muninn/neo4j/src/queries/consolidation.rs:90-93`) with order-independent set equality (`size(old_owners) = size(new_owners) AND ALL(x IN old_owners WHERE x IN new_owners)`).
  - Add live Neo4j regression test with multi-owner preferences inserted in opposite orders.
- **F-09 (Owner Filter Support):**
  - Add `user_identifier: Option<&str>` to `ConsolidationStore::detect_superseded_preferences` and `ConsolidationMemory::detect_superseded_preferences`, rejecting blank strings fail-closed and preserving strict cross-owner supersession rejection.
- **F-07 & `GM09-C11` (Production Composition Root & Public API Concurrency):**
  - Wire `Neo4jConsolidationStore` into `MemoryClient::with_consolidation(...)` in `apps/muninn/src/main.rs` and expose CLI/operator consolidation entry points defaulting to `--dry-run` (requiring explicit `--apply` for mutations).
  - Add `GM09-C11`: live Neo4j test verifying concurrent preference ownership changes through the public `MemoryClient` API.

### Out of Scope

- LLM-based trace summary text generation (upstream only flags `summarization_pending = true`).
- Background cron scheduling daemon for automatic consolidation.

## Approach

1. **TDD First:** Write failing unit tests for `report.validate_invariants()`, upfront `run_all()` validation (missing `ttl_days` aborts with zero store calls), upstream defaults, and blank `user_identifier` rejection; write failing live Neo4j tests for F-08 (multi-owner order insensitivity) and `GM09-C11` (public API concurrency).
2. **Core & Application:** Add `ConsolidationReport::validate_invariants()` in `muninn-core`, remove fallback `record_run` orchestration from `ConsolidationMemory`, update `ConsolidationOptions` & `run_all()` upfront validation, and add `user_identifier` to `ConsolidationStore`.
3. **Neo4j Adapter:** Update `FIND_SUPERSEDED_PREFERENCES` for set-based owner comparison and optional `user_identifier` anchoring; ensure every mutating job writes `:ConsolidationRun` atomically inside its transaction only when `mutations_applied > 0`.
4. **Production Composition Root:** Wire `with_consolidation` in `apps/muninn/src/main.rs` with `--dry-run` default.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/muninn/core/src/ports.rs` | Modified | `ConsolidationReport::validate_invariants()`, `user_identifier` parameter on `ConsolidationStore`. |
| `crates/muninn/application/src/consolidation.rs` | Modified | Remove fallback `record_run`, enforce invariants, align defaults, upfront `run_all()` validation. |
| `crates/muninn/neo4j/src/queries/consolidation.rs` & `consolidation.rs` | Modified | Order-insensitive owner set comparison (F-08), `user_identifier` filter (F-09), atomic audit invariants. |
| `apps/muninn/src/main.rs` | Modified | Wire `Neo4jConsolidationStore` in production composition root (F-07). |
| `crates/muninn/neo4j/tests/consolidation.rs` & `consolidation_queries.rs` | Modified | Tests for F-05, F-06, F-08, F-09, `GM09-C11`, `GM09-C12`. |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Updating defaults (`0.85 -> 0.95`, `min_steps 10 -> 20`, mandatory `ttl_days`) breaks existing Muninn tests assuming old defaults. | High | Update affected test fixtures explicitly and document the upstream alignment. |
| Mock/fake `ConsolidationStore` implementations in unit tests omit `run_id` when returning `mutations_applied > 0`. | Medium | Update test doubles to honor the strengthened port contract; `validate_invariants()` catches violations immediately. |
| CLI consolidation command accidentally mutates production graphs. | Medium | Default to `--dry-run` and require explicit `--apply` flag. |

## Rollback Plan

Revert the focused consolidation commits via `git revert`. No Neo4j schema changes or destructive data migrations are introduced.

## Dependencies

- `spec` can proceed in parallel with `SDD-1`; owner-scoping semantics align with `SDD-1D` (`F-04`).

## Success Criteria & Common Definition of Done

- [ ] Orchestrator fallback `record_run` removed; `ConsolidationReport::validate_invariants()` enforces single transactional audit ownership (F-05).
- [ ] `run_all()` validates full configuration (including mandatory `ttl_days`) upfront before executing any job (F-06).
- [ ] All consolidation defaults match upstream `memory/consolidation.py` (`GM09-C12`).
- [ ] `ConsolidationMemory` is wired into `apps/muninn/` production composition root with `--dry-run` default (F-07).
- [ ] `FIND_SUPERSEDED_PREFERENCES` owner comparison is order-insensitive (F-08) and supports `user_identifier` filtering (F-09).
- [ ] `GM09-C11` concurrent public API test passes under `MUNINN_REQUIRE_NEO4J=1`.
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
