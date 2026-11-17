# Design: Muninn Memory Evaluation and Benchmarking

## Technical Approach

This design covers only roadmap milestone 5 as authorized and specified in `specs/muninn-memory-evaluation/spec.md`: a minimal read-only MCP retrieval connector for messages and preferences, composed through an MCP command in `apps/muninn`; a deterministic synthetic Huginn task comparison; and one bounded local end-to-end (E2E) run after all gates pass. Entity retrieval and reasoning-trace retrieval are excluded because current retrieval APIs do not enforce owner scope for those types. Conformance, probabilistic component quality, end-to-end memory utility, and operational efficiency remain distinct. Security outcomes remain independent hard gates. No general benchmark runner, external dataset, paid/cloud provider, credential inspection, model download, destructive reset, production access, write/update/delete operation, or direct bridge/API fallback is in scope.

The selected integration route is Huginn's existing MCP stdio client calling the retrieval-only MCP server composed by a command in `apps/muninn`. Source inspection confirms Huginn's `rmcp` 3.8.0 client and Muninn's MCP implementation use revision `2026-07-28`. This is source compatibility only; a client/server integration test is still required. Muninn's current handler, dispatcher, and stdio `serve` loop are synchronous, while its application storage ports are asynchronous. The selected design changes the handler, dispatch, and stdio serving path to async end-to-end, awaits application operations directly, and processes requests sequentially. It uses bounded request/result sizes and an operation timeout. It does not use `block_on`, a nested runtime, an unbounded worker pool, or unbounded concurrency.

One immutable owner scope is configured explicitly by the host for each Muninn MCP server process. Startup fails if configuration is absent or invalid. Both message retrieval and preference retrieval MUST receive and enforce this exact same configured scope; neither path may derive, substitute, or override it. The retrieval requests accept no model-supplied scope selector. `user_identifier` selects a data scope; it does not authenticate a caller. This is a single-owner local connector, not a multi-tenant authentication mechanism. Huginn continues to require approval to start the MCP process and separate `HostPolicy` approval for each invocation. MCP output remains untrusted. The process runs without an OS sandbox and must be treated as a trusted process by its operator.

Technical choices are selected, but implementation and runtime readiness are not established. The approved composition point is an MCP command in `apps/muninn`; no claim is made that the standalone `muninn-mcp` binary is the composition root. Both messages and preferences must be wired to owner-scoped application retrieval operations using the same validated immutable host scope. No end-to-end integration test exists. The async conversion, composition, same-scope enforcement, and focused security/protocol tests are implementation prerequisites. The synthetic scenario, assertions, baseline semantics, claim scope, already-ready local model, and isolated disposable local resources also remain approval/readiness gates for the E2E run. No service startup, inference, or database operation occurs before those gates pass.

## Architecture Decisions

### Decision: Use a read-only MCP connector from Huginn to Muninn

**Choice**: Use Huginn's existing MCP runtime client to invoke read-only message and preference retrieval capabilities over stdio. Compose both Muninn application retrieval paths through an MCP command in `apps/muninn`. Do not add a Muninn dependency to Huginn core/application, direct bridge/API calls, or write/update/delete tools. Do not expose entity or reasoning-trace retrieval.

**Alternatives considered**: Direct bridge/API integration; placing Muninn dependencies in Huginn inner layers; a general adapter framework; treating the standalone `muninn-mcp` binary as the composition root or as a ready memory service.

**Rationale**: The user selected option A and specifically approved the `apps/muninn` command as the composition point. Current message and preference retrieval can be owner-scoped, while current entity and reasoning retrieval APIs do not enforce owner scope. Restricting the surface to the two enforceable paths avoids exposing unscoped data and avoids an implicit fallback with different authorization semantics.

### Decision: Make Muninn MCP handling asynchronous end-to-end and sequential

**Choice**: Convert `ToolHandler::call`, server dispatch, and stdio `serve` to async, and await the async application retrieval path directly. Keep one-request-at-a-time processing. Bound request and result sizes and enforce an operation timeout with fail-closed behavior. Do not introduce `block_on`, a nested runtime, worker-based bridging, or unbounded concurrency.

**Alternatives considered**: Keep a synchronous handler and block on async retrieval; add an unbounded worker/concurrency model; retain the synchronous `std::io` loop and defer/circumvent async retrieval.

**Rationale**: Muninn storage ports are async and the present synchronous MCP boundary cannot safely call them directly. Async propagation avoids nested execution runtimes; sequential handling preserves request order and bounds resource use. The exact size limits and timeout value must be selected during implementation and tested; this design does not set benchmark or product acceptance thresholds.

### Decision: Treat the protocol revision match as a prerequisite, not proof

**Choice**: Preserve MCP revision `2026-07-28` behavior and add an actual integration test that exercises client initialization/discovery and an approved retrieval invocation through the stdio server.

**Alternatives considered**: Treat matching source constants as sufficient; infer compatibility from isolated unit tests.

**Rationale**: Huginn `rmcp` 3.8.0 and Muninn source both implement revision `2026-07-28`, but this does not prove runtime interoperability, serialization, or tool-call behavior.

### Decision: Bind one host-configured owner scope per process

**Choice**: Resolve exactly one immutable owner scope from explicit host-owned process configuration before accepting requests. Reject startup when missing or invalid. Inject this exact immutable value into both message and preference retrieval paths and verify both operations enforce it. Do not expose a scope parameter to the model or permit a request to change the configured owner.

**Alternatives considered**: Accept a caller/model-provided `user_identifier`; infer owner identity from Huginn; treat the scope selector as authentication; add multi-tenant identity negotiation.

**Rationale**: Huginn does not supply authenticated user identity to MCP. Muninn's `user_identifier` is a data-scope selector, not authentication. Applying one host-validated scope identically to both supported retrieval operations avoids model-controlled cross-owner selection without claiming authentication that the integration does not provide. This does not make the MCP process sandboxed or multi-tenant.

### Decision: Preserve Huginn approval and untrusted-output boundaries

**Choice**: Retain Huginn's MCP process startup approval and per-invocation `HostPolicy` approval. Treat MCP result content as untrusted input. Document that the process has no OS sandbox and requires a trusted operator-controlled environment.

**Alternatives considered**: Let server configuration or tool metadata grant approval; treat local stdio as trusted data; claim approval provides process sandboxing.

**Rationale**: Huginn's startup permission and invocation permission are separate controls. Approval does not sandbox the process, and memory/tool output can contain instruction-like content. Keep authorization fail-closed and apply existing output handling.

### Decision: Compare a synthetic task with Muninn retrieval present or absent

**Choice**: Use synthetic non-sensitive inputs and a versioned task with deterministic observable assertions. Compare matched Huginn configurations that differ only in availability of the Muninn retrieval capability where feasible. Define baseline semantics precisely and control or disclose Huginn session history. Record task correctness and evidence use separately; do not use an LLM judge as the sole outcome source.

**Alternatives considered**: External benchmark datasets; production conversations; different models between arms; retrieval scores as a substitute for task outcomes; judge-only scoring.

**Rationale**: This is the narrow authorized milestone. Synthetic fixtures avoid external data and can support deterministic assertions, but one bounded run cannot establish statistical acceptance or generality. Huginn's own session history means omitting Muninn alone must not be mislabeled as “no memory.”

### Decision: Preserve secret-safe evidence and isolate writable state

**Choice**: If execution is approved, write a manifest, per-case outcomes, and a human-readable report to a new unique evidence directory. Verify a run-specific disposable Muninn database, Huginn session store, workspace, and output location before startup. Do not reset or reuse shared/persistent state. Keep efficiency observations separate and retain only necessary, secret-safe evidence.

**Alternatives considered**: Reuse configured/shared state; reset a persistent database; retain full prompts or protected memory payloads; overwrite prior evidence; blend efficiency and utility.

**Rationale**: The specification requires reproducible provenance, safe outputs, distinct statuses, and separate reporting dimensions. Isolation is a hard precondition, not a teardown assumption.

## Data Flow

```text
Huginn task
  └─ MCP process startup approval
      └─ `apps/muninn` MCP command validates one fixed host-configured owner scope
          └─ Huginn per-invocation HostPolicy approval
              └─ async sequential dispatch
                  └─ bounded read-only retrieval with timeout
                      └─ Muninn application retrieval service
                          ├─ message retrieval (fixed owner scope)
                          └─ preference retrieval (same fixed owner scope)
                              └─ scoped storage port(s)
                                  └─ bounded MCP result (untrusted to Huginn)
```

The separate evaluation flow runs only after explicit scenario/assertion/baseline/claim approval and local preflight:

```text
Versioned synthetic scenario + assertions
        ├─ Huginn with Muninn retrieval
        └─ Huginn baseline (Muninn retrieval unavailable; session history controlled/disclosed)
                 └─ observable task outcomes + evidence-use observations
                          └─ secret-safe per-case results, manifest, separate report
```

Preflight verifies an already-ready local model without credential inspection or downloads, the tested MCP route, required security and protocol tests, isolated disposable data stores/workspace/output, and required approvals. Any failed prerequisite blocks before service startup and inference. A security violation is reported as a gate failure regardless of task or efficiency results. Unmatched cases and deviations remain explicit; they are not converted into paired outcomes.

## File Changes

| File | Action | Description |
|------|--------|-------------|
| `crates/muninn/mcp/src/lib.rs` | Modify | Make handler and dispatch async; implement bounded sequential request handling and timeout behavior while preserving protocol revision behavior. |
| `apps/muninn` MCP command and command wiring | Modify | Compose message and preference retrieval through the MCP adapter; load and validate one immutable host-configured owner scope before serving and pass the identical scope to both retrieval paths. Exact CLI spelling and configuration detail remain implementation details. |
| Muninn MCP command/handler composition | Modify | Expose only bounded read-only message and preference retrieval. Do not treat the standalone `muninn-mcp` binary as the composition root; do not expose entities, reasoning traces, writes, updates, or deletes. |
| `crates/muninn/mcp/` tests | Modify | Add focused unit/contract tests for async dispatch, bounds, timeout, read-only tool shape, startup scope validation, and fail-closed behavior. |
| `crates/huginn/runtime/` MCP integration tests | Modify | Add an actual stdio client/server integration test for initialization/discovery and retrieval. Preserve existing startup and per-call approval behavior. |
| `.agents/rpi/plan/tasks/muninn-memory-evaluation-benchmarking/evidence/<unique-run-id>/` | Create, gated | Store the approved run manifest, raw per-case outcomes, and report only after all execution gates pass. |

The approved product composition point is an MCP command in `apps/muninn`. The exact command spelling and configuration surface may be resolved during implementation. Do not substitute the standalone `muninn-mcp` binary as the composition root, modify unrelated Huginn composition, or add bridge/API paths as a fallback. Verify that message and preference application retrieval each apply the same immutable host scope.

## Interfaces / Contracts

The MCP command in `apps/muninn` exposes only read-only message and preference retrieval. It exposes no entity retrieval, reasoning-trace retrieval, writes, updates, or deletes. The model-facing requests contain retrieval inputs only and cannot choose or override owner scope. The configured scope is immutable for the process lifetime, and the same scope is applied by both message and preference retrieval paths. Missing or invalid configuration prevents server startup. Retrieval uses Muninn application/storage contracts and remains asynchronous through handler, dispatch, and stdio serving. Request/result bounds and operation timeout must be enforced and covered by tests. No `block_on`, nested runtime, unbounded worker/concurrency, or scope fallback is permitted.

`user_identifier` is a data-scope selector and is not proof of identity or authentication. The process is a single-owner local connector. Huginn must retain separate MCP startup and per-invocation `HostPolicy` approval; result content is untrusted. The process is not OS-sandboxed.

Evaluation records must preserve scenario identity/version, approved conditions and baseline semantics, observable assertion outcomes, evidence-use observations, status/reason, deviations/unmatched cases, and available source/runtime/configuration provenance. Outputs must exclude secrets and protected payloads. No numeric acceptance threshold, sample size, repetition count, statistical method, confidence level, model, external dataset, or CI policy is selected here.

## Prerequisites and Execution Gate

The connector implementation is a prerequisite, not an unresolved design choice. Before an E2E run, the following must be verified:

1. Runtime MCP integration test passes for initialization/discovery and retrieval; source revision matching alone is insufficient.
2. Async handler/dispatch/stdio flow is implemented and tested with sequential processing, explicit request/result bounds, operation timeout, and no nested runtime or unbounded concurrency.
3. Scope startup validation, immutable binding, and cross-scope rejection are tested. The process scope comes from explicit host-owned configuration; it is not authentication and cannot be model-selected.
4. The MCP command in `apps/muninn` composes only message and preference retrieval. Tests prove both paths enforce the exact same host-configured immutable owner scope. Entity and reasoning-trace retrieval, write/update/delete capability, direct bridge/API integration, and fallback are absent.
5. Huginn startup approval, per-invocation `HostPolicy` approval, untrusted-output handling, and trusted-process/no-OS-sandbox disclosure are preserved.
6. The exact synthetic scenario, assertions, baseline semantics, evidence-use rule, security cases, and narrow claim are explicitly approved.
7. A compatible local model is already available and ready. Verify without credential inspection, model download, external network access, or cost-bearing service.
8. The Muninn database, Huginn session store, workspace, and output destination are unique, isolated, run-specific, and disposable. Do not reset or reuse shared/persistent data.
9. The unique evidence destination and secret-safe output handling are verified. Obtain required MCP server startup and per-call approvals.

If any item is missing, invalid, or requires an unapproved resource, stop before service startup and inference. Report the blocker; do not broaden authorization.

## Testing Strategy

| Layer | What to Test | Approach |
|-------|-------------|----------|
| Handler/dispatch | Async retrieval dispatch, sequential request processing, bounded input/output, timeout, errors | Focused deterministic tests with a fake application service/backend. |
| Scope/security | Missing/invalid startup scope; identical immutable scope enforcement for messages and preferences; rejection of model-supplied scope; cross-scope isolation; exclusion of entities/reasoning traces; read-only tool surface | Unit and contract tests for each retrieval path and shared configuration; fail closed and treat unauthorized disclosure as an independent gate failure. |
| Protocol integration | Huginn `rmcp` client initialization/discovery and retrieval against Muninn stdio server | Actual client/server integration test; source revision alignment is not a substitute. |
| Huginn approval/trust | Startup approval, per-call approval, denial/cancellation, untrusted MCP output | Tests at the existing runtime boundary; do not imply process sandboxing. |
| Evaluation assertions | Stable scenario identity/version, deterministic task assertions, evidence-use records, status and unmatched-case handling | Synthetic fixtures only; no inference required for connector tests. |
| Bounded E2E | Task outcomes with retrieval and baseline, provenance, security gates | Exactly one local run only after all approvals and preflight gates pass. |

Use TDD for executable behavior. No tests or inference are run as part of this design update. No numerical acceptance policy or CI gate is defined.

## Migration / Rollout

No data migration is required. The async MCP contract and `apps/muninn` command composition are implementation changes; maintain protocol revision behavior and fail startup on missing/invalid owner-scope configuration. Apply the identical immutable scope to message and preference retrieval. The E2E run is separately gated and additive. Any cleanup may target only explicitly identified disposable run resources after evidence preservation; shared or persistent state must not be reset or removed.

## Open Questions

- [ ] What exact synthetic scenario, observable assertions, evidence-use rule, security cases, baseline semantics, and narrow claim are approved?
- [ ] Which already-ready local model/provider can run without credential inspection, downloads, external access, or cost?
- [ ] Which verified disposable local database/backend, Huginn session store, workspace, and unique evidence destination will be used?
- [ ] What exact request/result bounds and operation timeout fit the retrieval contract? Select and test these during implementation; no evaluation threshold is implied.
- [ ] What non-numerical reporting details and any LLM-judge role, if any, are approved? A judge cannot be the sole outcome source.

Protocol revision matching, the MCP route, async strategy, and single-process scope model are selected technical designs. Runtime interoperability, implementation, security test results, service composition, backend wiring, scenario approval, local model readiness, and disposable-resource readiness remain prerequisites or open decisions; they are not design alternatives awaiting selection.
