# Proposal: Muninn `EvalMemory` Product API (`F-13`, `BOLT-08-07`)

## Intent

Implement the product-level `EvalMemory` API (`client.eval` and `client.eval.run`, `BOLT-08-07`, 2 eligible Bolt API rows) matching the pinned Neo4j Agent Memory contract (`9d45af5529ec`, `memory/eval.py`).

### Strict Conceptual Separation (Do Not Conflate)

This initiative owns **only** the product evaluation API and keeps three distinct evaluation concepts strictly separated:

1. **`EvalMemory` (this SDD — Product Feature):** Public API (`client.eval.run`) enabling callers to score retrieval relevance, audit completeness, preference fidelity, extraction quality, and resolution quality on a `MemoryClient`.
2. **H5 Acceptance Protocol (`.agents/sdd/changes/muninn-h5-acceptance-protocol`):** Retrieval subsystem acceptance (embedding identity, frozen corpus, ANN–exact offline protocol). Preserved untouched.
3. **Huginn Memory Benchmarking (`.agents/sdd/changes/muninn-memory-evaluation-benchmarking`):** End-to-end consumer agent utility comparison. Preserved untouched.

## Scope

### In Scope

- **Core Metric Primitives (`muninn-core`):**
  - Pure, deterministic scoring functions: `Recall@k`, Entity F1, Relation F1, and B-cubed F1 (matching `neo4j_agent_memory.core.metrics`).
  - Typed case & report models: `RetrievalCase`, `AuditCase`, `PreferenceCase`, `ExtractionCase`, `ResolutionCase`, `EvalReport` (with per-dimension results and explicit `skipped: Vec<String>`).
- **Application Orchestrator (`muninn-application::eval`):**
  - Implement `EvalMemory::run(...)` executing each requested dimension independently:
    1. **Retrieval relevance:** Recall@k against labeled `(query, expected_entity_ids)` cases.
    2. **Audit completeness:** Verify `(:Entity)<-[:TOUCHED]-(:ReasoningStep)` coverage for `(entity_id, expected_step_ids)`.
    3. **Preference fidelity:** Verify `get_preferences_for(active_only = true)` matches expected active preference IDs.
    4. **Extraction quality:** Run configured extractor and score Relation F1 (if gold relations given) or Entity F1.
    5. **Resolution quality:** Run configured resolver and score B-cubed F1 against gold clusters.
  - **Upstream Skipping vs. Release-Gate Enforcement:**
    - In the product API, if no extractor or resolver is configured on the client, cleanly skip `extraction` / `resolution` (leaving them `None` and recording their names in `EvalReport.skipped`) rather than raising — matching upstream `memory/eval.py`.
    - In release-gate validation (`SDD-1B` / acceptance runner), **a skipped mandatory dimension MUST NEVER count as `PASS`**.
- **Client & Bridge Wiring (`muninn-application`, `muninn-bridge`):**
  - Expose `client.eval()` on `MemoryClient` and wire `client.eval.run` into `muninn-bridge`.
- **Verification:**
  - Deterministic unit tests for all 5 metric calculators, configuration error cases, and clean dimension skipping.
  - Live Neo4j integration test (`crates/muninn/neo4j/tests/eval_memory.rs`, `MUNINN_REQUIRE_NEO4J=1`) verifying retrieval, audit completeness, and preference fidelity against controlled fixtures.

### Out of Scope

- Replacing or modifying `muninn-h5-acceptance-protocol` or `muninn-memory-evaluation-benchmarking`.
- External benchmark datasets, paid cloud LLM judges, or statistical release thresholds (handled separately).

## Approach

1. Implement pure metric calculators in `muninn-core` using strict TDD against reference test vectors from upstream `core/metrics.py`.
2. Build `EvalMemory` in `muninn-application/src/eval.rs` depending strictly on existing ports (`LongTermStore`, `ReasoningStore`, `EntityExtractor`, `EntityResolver`).
3. Wire `client.eval()` on `MemoryClient` and expose over `muninn-bridge`.
4. Verify live Neo4j behavior on controlled fixtures and ensure the release-gate validator rejects skipped mandatory dimensions.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/muninn/core/src/metrics.rs` & `model.rs` | New / Modified | Pure Recall@k, Entity/Relation F1, B-cubed F1, and `EvalReport` types. |
| `crates/muninn/application/src/eval.rs` & `client.rs` | New / Modified | `EvalMemory` orchestrator and `client.eval()` accessor. |
| `crates/muninn/bridge/src/` | Modified | Bridge binding for `client.eval.run`. |
| `crates/muninn/neo4j/tests/eval_memory.rs` | New | Live Neo4j tests for `BOLT-08-07`. |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| `EvalMemory` output is misreported as overall parity score or H5 acceptance. | High | Document explicitly in types/docs that `EvalReport` is a product diagnostic, not a parity score or H5 acceptance; release gate rejects skipped mandatory dimensions. |
| Metric rounding or tie-breaking diverges from upstream `core/metrics.py`. | Medium | Cross-check unit tests directly against pinned upstream `core/metrics.py` cases. |

## Rollback Plan

Purely additive module; revert via `git revert` without affecting storage, schema, or existing evaluation SDDs.

## Dependencies

- Coordinates with `SDD-1B` (evidence validator must reject skipped mandatory dimensions) and remains strictly independent of `muninn-h5-acceptance-protocol` and `muninn-memory-evaluation-benchmarking`.

## Success Criteria & Common Definition of Done

- [ ] `client.eval` and `client.eval.run` (`BOLT-08-07`) implemented across all 5 dimensions with exact upstream skipping behavior.
- [ ] Release-gate check confirms skipped mandatory dimensions never count as `PASS`.
- [ ] Zero overlap or modification to `muninn-h5-acceptance-protocol` or `muninn-memory-evaluation-benchmarking`.
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
