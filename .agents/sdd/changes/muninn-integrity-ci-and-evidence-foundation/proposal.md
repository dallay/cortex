# Proposal: Muninn Integrity, CI Stability, and Evidence Foundation

## Intent

Establish an honest, reproducible engineering, security, and CI baseline for Muninn before claiming functional parity progress. Code verification confirms foundational defects from the Revision 4 Parity Audit across CI stability, evidence integrity, transactional ingestion, and multi-owner authorization.

To avoid an oversized P0 monolith, this SDD serves as the **coordinating foundation** split into **four independent delivery units (`1A`, `1B`, `1C`, `1D`)** that can be specified, reviewed, and merged incrementally.

## Delivery Units & Scope

### Unit 1A — CI Stability (`F-10`, `F-11`, `F-21`)

- **F-11:** Gate `PinnedMiniLmManifest::synthetic_oversized_for_tests` and `synthetic_for_tests` in `crates/muninn/c1-exporter/src/model_manifest.rs` with `#[cfg(all(test, unix))]` matching their Unix-only callers so Windows Clippy `-D warnings` passes.
- **F-10:** Fix or explicitly isolate Windows Docker fixture execution in `crates/muninn/neo4j/tests/consolidation.rs` with deterministic diagnostics instead of opaque `docker run failed`.
- **F-21:** Triage and resolve the OSV-Scanner CI workflow failure.

### Unit 1B — Evidence & Traceability Integrity (`F-18`, `F-19`, `F-20`)

- **F-18:** Partition the 19 `#[ignore]` live Neo4j integration targets in `crates/muninn/neo4j/tests/` into isolated-fixture `just` recipes and CI jobs running under `MUNINN_REQUIRE_NEO4J=1` without cross-test database collisions.
- **F-19 & F-20:** Replace manual parity closure claims with an automated, commit-bound evidence validator (`dev/muninn/`) that:
  - Ensures `docs/muninn/specs/api-test-traceability.md` and `api-test-traceability.json` are generated from a single source of truth and stay identical.
  - Rejects missing test file references (such as the non-existent `crates/muninn/bridge/tests/buffered.rs`), stale commit SHAs, unknown capability IDs, and skipped/ignored tests counted as `PASS`.
  - Tracks all four release metrics simultaneously: `132/138` API ops, `88/92` independent capabilities, `27/27` critical criteria, and `178/178` selected TCK.

### Unit 1C — Transactional Ingestion (`F-01`)

- **Single Neo4j Transaction (No Outbox):** Execute `append_message` and extracted entity/mention/relation writes inside a **single Neo4j transaction** (`neo4rs::Txn`) rather than introducing unnecessary transactional-outbox infrastructure when all mutations target the same store.
- Run LLM/extractor `prepare()` *outside* the database transaction so DB lock duration stays minimal.
- Explicitly specify and test edge cases:
  1. Extraction write failure rolls back the message write completely (zero orphan messages).
  2. Concurrent retries with the same `operation_key` and identical payload are idempotent.
  3. Reusing an `operation_key` with a different payload or conversation scope fails closed with `MemoryError::invalid`.
  4. Connection loss *after* server-side commit is safely resolved by client retry with the same `operation_key`.

### Unit 1D — Authorization & Owner Isolation Policy (`F-04`)

- **Memory Classification Policy First:** Do **not** blindly impose `OwnerScope` onto every entity (which would break shared graph knowledge). Define and enforce an explicit scope-classification contract in `muninn-core`:

| Memory Kind | Scope Classification | Access Rule |
|-------------|----------------------|-------------|
| Conversations & messages | **Private** | Strict `OwnerScope` isolation; zero cross-owner reads/writes/deletes. |
| Personal preferences | **Private** | Strict `OwnerScope` isolation; cross-owner supersession rejected. |
| Reasoning traces, steps & tool calls | **Private** | Strict `OwnerScope` isolation across read/write/search/list. |
| Shared knowledge (entities/facts in workspace) | **Workspace** | Scoped to authorized workspace boundary; explicit opt-in sharing. |
| Public reference entities | **Global** | Read-shared reference nodes; mutations restricted to authorized policy. |
| Audit records (`ConsolidationRun`, `MemoryReadAudit`) | **Restricted / administrative** | Never exposed to ordinary user retrieval; restricted to operator/admin boundary. |

- Audit all `ShortTermStore`, `LongTermStore`, `ReasoningStore`, `ConsolidationStore`, and `LocalOperator` entry points against this classification and add adversarial cross-tenant regression tests.

### Out of Scope

- Implementing buffered writes (`BOLT-08-04`), existing graph adoption (`BOLT-08-06`), consolidation feature changes, MCP new tools, `EvalMemory`, spatial retrieval, or H5 real-model benchmarks (owned by dedicated SDDs).

## Approach

1. Land **Unit 1A** immediately to restore green cross-platform CI (`cargo clippy`, Windows test handling, OSV-Scanner).
2. Land **Unit 1B** to establish automated traceability/closure validation and isolated live Neo4j execution recipes.
3. Specify and implement **Unit 1C** (single-transaction ingestion + `operation_key` concurrency/post-commit retry tests) and **Unit 1D** (classification policy + adversarial isolation suite) via strict Red → Green → Refactor.

## Affected Areas

| Area | Unit | Impact | Description |
|------|------|--------|-------------|
| `crates/muninn/c1-exporter/src/model_manifest.rs` | 1A | Modified | `#[cfg(all(test, unix))]` on Unix-only test helpers. |
| `crates/muninn/neo4j/tests/consolidation.rs` & `.github/workflows/` | 1A | Modified | Windows fixture handling and OSV-Scanner fix. |
| `dev/muninn/`, `docs/muninn/specs/`, `justfile` | 1B | Modified | Automated traceability/parity validator and isolated Neo4j test matrix. |
| `crates/muninn/core/src/ports.rs`, `application/src/short_term.rs`, `neo4j/src/` | 1C | Modified | Single-transaction message + extraction commit boundary and retry semantics. |
| `crates/muninn/core/src/ports.rs`, `application/src/`, `neo4j/src/` | 1D | Modified | Explicit memory scope classification and uniform authorization enforcement. |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Imposing `OwnerScope` blindly breaks shared entity graph traversal or TCK expectations. | High (if unclassified) | Unit 1D explicitly classifies Private vs Workspace vs Global vs Restricted before changing queries. |
| Enabling 19 ignored Neo4j integration tests slows CI or causes fixture collisions. | Medium | Isolate each test target with dedicated ephemeral fixtures/namespaces and partition CI matrix jobs. |
| Single-transaction ingestion increases lock contention under high concurrency. | Low | Keep external embedding/extraction calls outside `neo4rs::Txn`; keep only Cypher writes inside the transaction. |

## Rollback Plan

Each unit (`1A`, `1B`, `1C`, `1D`) ships in independent, atomic PRs and can be reverted individually via `git revert` without affecting persisted Neo4j schemas or historical TCK receipts.

## Dependencies

- None (this is the P0 foundation; `SDD-2`..`SDD-5` may draft `spec`/`design` in parallel, with production integration conditioned on `1C`/`1D` where applicable).

## Success Criteria & Common Definition of Done

- [ ] Unit 1A: Windows Clippy (`muninn-c1-exporter`), Windows consolidation fixture handling, and OSV-Scanner are green.
- [ ] Unit 1B: Automated validator proves `api-test-traceability.md` == `api-test-traceability.json`, rejects missing test paths or ignored tests counted as `PASS`, and executes all in-scope Neo4j integration targets under `MUNINN_REQUIRE_NEO4J=1`.
- [ ] Unit 1C: Message + extraction persistence runs in one Neo4j transaction with passing tests for rollback on extraction failure, concurrent `operation_key` retries, conflicting payload rejection, and post-commit disconnect recovery.
- [ ] Unit 1D: Memory scope classification policy is documented in `muninn-core` and enforced with zero cross-tenant leakage in adversarial tests.
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
