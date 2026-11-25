# Proposal: Muninn Existing Graph Adoption (`BOLT-08-06`)

## Intent

Implement `adopt_existing_graph` (`BOLT-08-06`, Critical Acceptance Criterion #27) matching and strengthening the pinned Neo4j Agent Memory contract (`9d45af5529ec`, `graph/schema.py:500-566`). Currently, `SchemaManager` in `crates/muninn/neo4j/src/schema.rs` handles fresh schema installation (`setup_all`), but has no `adopt_existing_graph` operation to non-destructively onboard pre-existing Neo4j domain graphs into Muninn's `:Entity` memory model. Completing this closes **F-03** and differential scenario **GM-10**.

## Scope

### In Scope

- **Four-Stage Adoption Pipeline (`Validate → Plan → Apply → Verify`):**
  1. **Validate:** Validate *every* label and `name_property` upfront against `^[A-Za-z_][A-Za-z0-9_]*$` (all-or-nothing, zero DB calls on invalid input) and enforce administrative/write authorization before touching the graph. (Strengthens upstream, which validates iteratively per label.)
  2. **Plan:** Inspect candidate nodes, already-adopted nodes, skipped null-name nodes, and **pre-flight conflicts** (duplicate `id` collisions with `entity_id` constraint, incompatible property types, multi-label overlap). In `dry_run = true` (default), return `AdoptionReport` with zero mutations.
  3. **Apply:** Execute transactional, non-destructive adoption adding `:Entity` and populating missing Muninn keys (`id`, `name`, `canonical_name`, `type`, `name_key`, `surface_keys`, adoption provenance marker) via `coalesce(...)` while preserving all existing domain labels, properties, and relationships.
  4. **Verify:** Confirm postconditions (adopted counts, constraint integrity, zero lost relationships/properties) and emit structured `AdoptionReport` (`by_label: Vec<AdoptionLabelReport>`, `dry_run: bool`, conflict diagnostics).
- **Domain Models, Ports & Surface (`muninn-core`, `muninn-neo4j`, `muninn-application`, `muninn-bridge`):**
  - Define `AdoptGraphOptions`, `AdoptionLabelReport`, `AdoptionReport`, identifier validator, and `SchemaAdoptionPort` in `muninn-core`.
  - Implement in `SchemaManager::adopt_existing_graph` (`muninn-neo4j`), expose on `MemoryClient` (`muninn-application`), and wire into `muninn-bridge` for `GM-10`.
- **Mandatory Acceptance Scenarios (Unit + Live Neo4j `MUNINN_REQUIRE_NEO4J=1`):**
  - Cypher injection rejection (backticks, spaces, semicolons, empty strings, control chars) with zero DB queries.
  - Pre-existing `id` conflicts against existing `:Entity` nodes or duplicate `id`s among candidates.
  - Nodes carrying **multiple eligible labels** in the same adoption request (deterministic handling without double-counting or corrupting `type`).
  - Incompatible property types (e.g., non-string `name_property` or conflicting `id` type).
  - Property collisions (preserving pre-existing domain properties without silent overwrite).
  - Concurrent modifications during adoption.
  - Relationship preservation (all pre-existing inbound/outbound domain edges intact).
  - Partial-failure recovery (transactional rollback leaving zero half-adopted nodes on error).
  - Idempotent re-run (`migrated_count == 0`, all previously adopted nodes in `already_adopted_count`) and post-adoption readability via `LongTermMemory`.

### Out of Scope

- Destructive schema drops or automatic vector index rebuilds.
- Automatic embedding generation during adoption (callers invoke embedding backfill explicitly).
- Transferring ownership of already-managed private Muninn `:Entity` nodes across tenants.

## Approach

1. **Security First:** Validate all identifiers and authorization upfront in `muninn-core` before any Cypher is built or sent.
2. **Conflict-Aware Planning:** Run read-only conflict detection queries before applying mutations so `dry_run = true` accurately surfaces both counts and blockers.
3. **Transactional Apply + Provenance Marker:** Apply mutations within explicit Neo4j transactions and tag newly adopted nodes with non-destructive adoption metadata (e.g., `adopted_at` / `adopted_from_label`) so operators can audit or reverse an adoption cleanly if needed.
4. **Differential Parity (`GM-10`):** Compare Muninn and upstream `SchemaManager.adopt_existing_graph` on equivalent pre-existing fixtures.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/muninn/core/src/model.rs` & `ports.rs` | Modified | `AdoptGraphOptions`, `AdoptionReport`, identifier validation, `SchemaAdoptionPort`. |
| `crates/muninn/neo4j/src/schema.rs` & `queries/` | Modified | 4-stage `adopt_existing_graph` implementation and Cypher builders. |
| `crates/muninn/application/src/client.rs` | Modified | Expose schema adoption on `MemoryClient` (write-authorized only). |
| `crates/muninn/bridge/src/` | Modified | Wire `schema.adopt_existing_graph` for `GM-10` differential harness. |
| `crates/muninn/neo4j/tests/graph_adoption.rs` | New | Live Neo4j acceptance tests covering all conflict, multi-label, concurrency, and recovery scenarios. |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Cypher injection through label or property names. | High (if unvalidated) | Upfront ASCII identifier regex validation (`^[A-Za-z_][A-Za-z0-9_]*$`) before any query runs. |
| Nodes with multiple eligible labels get inconsistent `type` or double-counted. | Medium | Specify deterministic label precedence and track already-adopted nodes within the transaction. |
| Large graph adoption hits Neo4j transaction memory limits. | Medium | Document batching boundaries and fail cleanly with transactional rollback rather than partial corruption. |

## Rollback Plan

Distinguish three independent rollback layers (reverting Rust code does **not** undo Neo4j database mutations):

1. **Software Rollback:** Revert the Rust commits (`muninn-core`, `muninn-neo4j`, `muninn-application`, `muninn-bridge`) via `git revert`. Existing `setup_all` and memory APIs remain unaffected.
2. **Transactional Rollback (Runtime Failure):** Any constraint violation or mid-apply error aborts the Neo4j transaction, leaving the graph unmodified.
3. **Data Recovery (Previously Committed Adoption):** Because adoption is additive and tags adopted nodes with adoption metadata while preserving original labels and properties, operators can recover a pre-adoption state via a documented recovery procedure (e.g., removing `:Entity` and adoption-only properties from nodes matching the adoption run marker, or restoring from a pre-adoption Neo4j backup/snapshot — recommended before production `dry_run = false`).

## Dependencies

- `spec` can proceed in parallel with `SDD-1`; production authorization enforcement aligns with `SDD-1D` (`F-04`).

## Success Criteria & Common Definition of Done

- [ ] Four-stage pipeline (`Validate → Plan → Apply → Verify`) implemented; invalid identifiers fail closed with zero DB calls.
- [ ] All acceptance scenarios pass (ID conflicts, multi-label nodes, incompatible types, property collisions, concurrency, relationship preservation, partial-failure rollback, idempotent re-run).
- [ ] Rollback plan documents software rollback, transactional rollback, and data recovery for committed adoptions.
- [ ] Contract compared with the pinned upstream implementation (`9d45af5529ec`).
- [ ] Approved deviations explicitly documented.
- [ ] Hexagonal architecture boundaries preserved.
- [ ] RED → GREEN → REFACTOR evidence recorded.
- [ ] Unit and contract tests passed.
- [ ] Mandatory isolated Neo4j integration tests passed (`MUNINN_REQUIRE_NEO4J=1`).
- [ ] Concurrency, cancellation and failure paths verified.
- [ ] Owner isolation and authorization verified.
- [ ] Differential acceptance scenarios executed (`GM-10`).
- [ ] Bazel and Cargo build inventory synchronized.
- [ ] Documentation and evidence registry synchronized.
- [ ] Acceptance receipts bound to the candidate commit.
- [ ] No skipped or ignored scenario counted as passed.
