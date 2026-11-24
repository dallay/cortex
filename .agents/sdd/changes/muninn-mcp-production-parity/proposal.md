# Proposal: Muninn MCP Production Parity (`F-12`, `GM-12`)

## Intent

Complete production MCP interoperability for Muninn matching the pinned Neo4j Agent Memory MCP server contract (`9d45af5529ec`, `mcp/server.py`) — exposing the 6 core tools, 16 extended Bolt tools, resources/prompts, and Streamable HTTP transport — while strictly preserving the existing read-only `Catalog::ScopedRetrieval` profile required by `.agents/sdd/changes/muninn-memory-evaluation-benchmarking` (Milestone 5).

Currently, `apps/muninn/src/mcp_command.rs:171-193` registers only 2 read-only tools (`muninn_search_messages`, `muninn_search_preferences`) over `stdio`.

## Scope

### In Scope

- **Profile Separation (Zero Regression to Milestone 5):**
  - Preserve `Catalog::ScopedRetrieval` (read-only 2-tool profile) completely untouched for Huginn Milestone 5 (`muninn-memory-evaluation-benchmarking`), retaining host-controlled immutable owner scope, startup/per-call approvals, untrusted-memory handling, bounded limits, and fail-closed behavior.
  - Add `--profile core` (6 tools): `memory_search`, `memory_get_context`, `memory_store_message`, `memory_add_entity`, `memory_add_preference`, `memory_add_fact`.
  - Add `--profile extended` (16 tools): core tools plus reasoning trace/step/tool-call tools, entity relationship/merge/graph export tools, and MCP resources/prompts wiring.
- **Security & Authorization Boundary:**
  - Preserve host-bound immutable `HostOwnerScope`; reject any model-supplied `user_identifier` / tenant override fail-closed.
  - Align with `SDD-1D` memory scope classification (`Private` / `Workspace` / `Global` / `Restricted`).
  - Keep write profiles explicitly opt-in at the composition root.
- **Transport Parity (`stdio` + Streamable `http`):**
  - Retain `stdio` as the default transport.
  - Add Streamable HTTP (`--transport http` / `streamable-http`) in `muninn-mcp` + `apps/muninn`.
  - **Authentication Gate (Open Decision #4):** Default to loopback-only binding (`127.0.0.1`); require explicit authentication (recommended: OAuth 2.1 + host-controlled scope mapping, or explicit loopback-only local token mode) before allowing non-loopback HTTP binding. Reject legacy HTTP+SSE or normalize with explicit deprecation warning per spec.
- **End-to-End Verification (`GM-12`):**
  - Protocol discovery, tool invocation, authorization rejection, persistence round-trip, context retrieval, error propagation, and graceful shutdown against live Neo4j (`MUNINN_REQUIRE_NEO4J=1`).

### Out of Scope

- Modifying or weakening `Catalog::ScopedRetrieval` or Huginn Milestone 5 evaluation harnesses.
- NAMS hosted-only MCP tools (excluded from first Bolt release).
- `EvalMemory` API, spatial retrieval, or extraction pipeline changes (owned by their own SDDs).

## Approach

1. Keep `Catalog::ScopedRetrieval` isolated behind its own registration path and regression suite.
2. Implement `core` (6 tools) first over `stdio` with strict argument schema validation, bounded limits, and `HostOwnerScope` enforcement.
3. Extend to `extended` (16 tools) + resources/prompts.
4. Add Streamable HTTP transport only after confirming the auth contract (Decision #4: OAuth 2.1 + host-controlled scope vs local-only static token vs deferred HTTP).

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/muninn/mcp/src/` | Modified | Tool catalog profiles (`ScopedRetrieval`, `Core`, `Extended`), resources/prompts, Streamable HTTP transport & auth middleware. |
| `apps/muninn/src/mcp_command.rs` & `main.rs` | Modified | CLI `--profile` and `--transport` wiring while preserving `ScopedRetrieval` unchanged. |
| `crates/muninn/mcp/tests/` & `apps/muninn/` | Modified | Protocol, authorization, cross-tenant rejection, and live `GM-12` integration tests. |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Adding write MCP tools inadvertently regresses Huginn Milestone 5 read-only guarantees. | Medium | Separate `ScopedRetrieval` catalog + dedicated non-regression tests asserting zero write tools when `ScopedRetrieval` is selected. |
| Streamable HTTP exposes private memory without strong auth. | High | Bind `127.0.0.1` by default and fail closed on non-loopback without configured auth/TLS and host-bound owner scope. |
| Model-supplied arguments attempt to override tenant scope. | High | Reject unknown/scope-override keys at argument validation (same strict allowlist pattern already in `mcp_command.rs`). |

## Rollback Plan

Revert MCP profile and transport additions via `git revert`; `Catalog::ScopedRetrieval` over `stdio` remains intact and unaffected.

## Dependencies

- Authorization/scope rules depend on `SDD-1D` (`F-04`).
- Production composition-root integration sequences after `SDD-2` and `SDD-4` on `apps/muninn/src/main.rs`.
- Coordinates with (does not modify) `.agents/sdd/changes/muninn-memory-evaluation-benchmarking`.

## Success Criteria & Common Definition of Done

- [ ] `Catalog::ScopedRetrieval` passes all existing Milestone 5 read-only and fail-closed tests without modification.
- [ ] `core` (6 tools) and `extended` (16 tools) profiles match upstream MCP contracts and enforce immutable `HostOwnerScope`.
- [ ] Streamable HTTP transport enforces confirmed auth policy and fails closed when unauthenticated.
- [ ] Contract compared with the pinned upstream implementation (`9d45af5529ec`).
- [ ] Approved deviations explicitly documented.
- [ ] Hexagonal architecture boundaries preserved.
- [ ] RED → GREEN → REFACTOR evidence recorded.
- [ ] Unit and contract tests passed.
- [ ] Mandatory isolated Neo4j integration tests passed (`MUNINN_REQUIRE_NEO4J=1`).
- [ ] Concurrency, cancellation and failure paths verified.
- [ ] Owner isolation and authorization verified.
- [ ] Differential acceptance scenarios executed (`GM-12`).
- [ ] Bazel and Cargo build inventory synchronized.
- [ ] Documentation and evidence registry synchronized.
- [ ] Acceptance receipts bound to the candidate commit.
- [ ] No skipped or ignored scenario counted as passed.
