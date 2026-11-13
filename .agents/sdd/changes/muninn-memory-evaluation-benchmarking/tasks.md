# Tasks: Muninn Memory Evaluation and Benchmarking — Milestone 5

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Review budget | 400 changed lines |
| Estimated workload | High; async MCP changes, app composition, Huginn runtime integration, security tests, and separately gated evaluation span multiple surfaces. |
| Chained PRs recommended | No |
| Delivery strategy | One local feature branch from `muninn@09bb21d5`; no push or PR now. The user deferred any PR until Muninn exceeds 90% parity with Agent Memory. |
| Work-unit balance | Keep the dependency-ordered implementation and verification units below reviewable within the single local branch. These units do not authorize separate PRs. |

Decision needed before apply: No
Chained PRs recommended: No
400-line budget risk: High

The user approved one local feature branch based on `muninn@09bb21d5`, with no push or PR at this stage. Do not create a PR until Muninn is above 90% parity with Agent Memory; the parity measure is not defined here and must not be invented. Scenario and run-readiness approvals gate only E2E startup and inference, not connector implementation.

## Phase 1: Compatibility and contract inspection (complete)

- [x] 1.1 Inspect `crates/muninn/mcp/src/lib.rs` and Huginn `rmcp` 3.8.0 contracts; record the source revision match `2026-07-28` without claiming runtime interoperability (evidence: `apply-progress.md`, Phase 1).
- [x] 1.2 Map handler, dispatch, and stdio `serve` async propagation and internal call sites; confirm sequential async handling is locally contained and requires no `block_on`, nested runtime, worker bridge, or unbounded concurrency (evidence: `apply-progress.md`, Phase 1).
- [x] 1.3 Inspect async application ports, owner-scoped retrieval, `apps/muninn` composition, Huginn startup/per-call approvals, and untrusted output; record why entity/reasoning retrieval is excluded (evidence: `apply-progress.md`, Phase 1).

## Phase 2: Async bounded MCP and owner scope (work unit 1; TDD)

- [x] 2.1 RED: Add focused tests in `crates/muninn/mcp/` for async handler/dispatch/stdio requests, sequential ordering, protocol metadata, malformed input, retrieval-only tool surface, and request/result bounds.
- [x] 2.2 GREEN: Convert `ToolHandler::call`, server dispatch, and `serve` in `crates/muninn/mcp/src/lib.rs` to async end-to-end; await handlers directly, preserve revision behavior, and process one bounded request at a time.
- [x] 2.3 REFACTOR: Keep async flow simple and sequential; verify no `block_on`, nested runtime, blocking bridge, worker pool, or unbounded concurrency is introduced.
- [x] 2.4 RED: Add tests for missing/invalid host scope rejection, immutable process scope, absence of request/model scope selectors, and cross-owner denial for both supported retrieval paths.
- [x] 2.5 GREEN: Add one validated immutable host-configured owner scope at the `apps/muninn` MCP command boundary; fail startup closed when absent or invalid, and pass the same scope to message and preference retrieval. Treat `user_identifier` only as a selector, never authentication.
- [x] 2.6 RED/GREEN/REFACTOR: Test and implement bounded message/preference queries, bounded results, operation timeout, errors, and fail-closed behavior in `crates/muninn/mcp/` and app composition; ensure neither path substitutes or overrides the configured scope.

## Phase 3: `apps/muninn` retrieval composition and Huginn protocol (work unit 2; TDD)

- [x] 3.1 RED: Add `apps/muninn` command/composition tests proving only read-only messages and preferences are exposed and both invoke owner-scoped application retrieval with the exact same immutable host scope.
- [x] 3.2 GREEN: Compose the MCP command in `apps/muninn` with existing Muninn application/storage adapters for messages and preferences. Do not use standalone `muninn-mcp` as composition root; exclude entities, reasoning traces, writes, updates, deletes, and direct bridge/API fallback.
- [x] 3.3 RED: Add an actual Huginn `crates/huginn/runtime/` MCP client/server integration test for initialization, discovery, approved retrieval, denial/error paths, and output handling; source revision matching alone is insufficient. The protocol test uses duplex IO to avoid Neo4j; existing runtime transport tests separately cover the child-process stdio path.
- [x] 3.4 GREEN/REFACTOR: Wire the testable command/server route while preserving Huginn MCP startup approval and per-invocation `HostPolicy` approval. Treat returned memory/tool content as untrusted; retain disclosure that the process has no OS sandbox.
- [x] 3.5 Verify focused Muninn and affected Huginn tests, formatting, relevant Clippy with warnings denied, and `just huginn-architecture` if Huginn core/application contracts change; record actual results and do not claim runtime compatibility before the integration test passes.

## Phase 4: Approved synthetic comparison and gated local E2E (work unit 3)

- [x] 4.1 Define and obtain explicit approval for the exact versioned synthetic scenario, observable assertions, evidence-use/security cases, baseline semantics, session-history treatment, and narrow claim. Approved protocol: `synthetic-e2e-proposal.md`. Do not use an LLM judge as the sole outcome source; no statistical or numerical acceptance policy is authorized.
- [x] 4.2 Before startup or inference, verify a compatible already-ready local model/provider without inspecting credentials, downloading artifacts, using external network access, or incurring cost; verify unique isolated disposable Muninn DB, Huginn session store, workspace, and new evidence destination. Never reset or reuse persistent/shared state. Run evidence records local Ollama/Qwen, MiniLM, pinned Neo4j, tmpfs-only DB state, loopback Bolt binding, and isolated session/workspace copies.
- [x] 4.3 Verify protocol interoperability and all async, bounds/timeout, same-scope authorization/isolation, read-only surface, approval, and untrusted-output tests. Obtain Huginn server-start and each required invocation approval. If any gate fails, stop before startup/inference and record the blocker. Focused pre-run verification passed; live Huginn startup, each MCP invocation, and each file write used normal `HostPolicy` approvals.
- [x] 4.4 Only after every gate passes, perform exactly one bounded local synthetic comparison with Muninn retrieval available/unavailable; hold provider/model, tools, prompt, context budget, workspace fixture, and permissions constant where feasible, and disclose baseline/session-history semantics and deviations. The one pair ran with matching model, prompt, 32768 context budget, and fixture. The baseline passed; the memory condition failed the preference/title/columns assertions.
- [x] 4.5 Retain only secret-safe manifest, per-case outcomes, and report under `.agents/rpi/plan/tasks/muninn-memory-evaluation-benchmarking/evidence/<unique-run-id>/`; record provenance, statuses, assertions, evidence use, unmatched cases, deviations, and independent security gates. Keep efficiency separate; add no general runner, external dataset/labels, broad statistical/CI framework, or numeric acceptance values. Evidence is in `synthetic-memory-recall-v1-20261110T101934Z-f8c62b5e/`; vectors and credentials are omitted from the sanitized session evidence.
