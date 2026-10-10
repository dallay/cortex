# ADR-0012: Hexagonal application and lifecycle-owned capabilities

- Status: Accepted
- Date: 2026-10-11
- Product: Huginn
- Decision basis: Architectural audit and explicit direction to implement all four incremental refactoring phases.
- Refines: ADR-0003 and ADR-0010.

## Context

The service kernel already stages services and supervises plugin tasks, but the
standard loop shared a crate with HTTP, SQLite and MCP adapters. Native tools were
constructed as a registry and MCP discovery mutated that registry outside plugin
activation. Partial discovery failures could leave stale tools. Presentation also
implemented permission policy and interpreted invocation grants.

## Decision

Huginn is a hexagonal application composed by a microkernel of statically linked
plugins. Every non-kernel capability implements an inward-facing port and
participates in plugin lifecycle. Application use cases depend exclusively on
contracts and ports, never concrete adapters. The composition root assembles
concrete implementations.

Enforce the useful Cargo boundary first, without creating a crate per tool:

- `huginn-core`: domain/session/event models, ports and generic plugin kernel.
- `huginn-application`: standard agent loop, context/recovery/compaction policies,
  output budgets, host authorization and port-only service composition.
- `huginn-runtime`: HTTP/SSE, native tools, MCP, SQLite and filesystem instructions;
  adapter factories and built-in plugin assembly.
- `huginn-presentation`: terminal driver, interaction and declarative contributions.
- `apps/huginn`: configuration, invocation grants, session ownership and composition.

`InstructionSource` is injected into the loop like model, tools and sessions. The
filesystem adapter owns blocking I/O. Removing its service stops dependent loops.
Pure output truncation is application policy shared with adapters. Approval header
and effect summaries belong to presentation, outside core.

Each native tool is a plugin. Each MCP server is a plugin that discovers tools
before publication and owns its client shutdown task. The tool registry exposes
an adapter registration service separately from its inward-facing lookup port.
The production registry plugin supplies both services, so its removal stops
consumers and revokes native/MCP capabilities.

`PluginContext::own_resource` owns leases even if activation fails before
`deactivate` can run. `on_commit` publishes staged adapter registrations after
successful activation and manifest validation. A server's tool batch becomes
visible under one registry lock. Kernel cleanup revokes leases before waiting for
managed work. Registrations carry generation identity; retained tool handles
reject preparation/execution after revocation. Revocation cancels in-flight adapter
futures; it cannot undo already completed external effects.

Presentation implements `ApprovalPrompt` to collect a fresh decision, without
receiving invocation grants. `HostPolicy` is the sole `ApprovalPolicy` used by the
production loop and MCP startup. It applies exact action grants, otherwise requests
a user decision, and fails closed on cancellation or unavailable presentation.
Both terminal adapters retain their existing confirmation behavior and numbering.
Native presentation code remains trusted: a Rust trait is not a security sandbox
for arbitrary third-party machine code.

Provider factories are registered outside CLI bootstrap and validated through the
same registry. Extending provider construction changes neither loop nor kernel.
Service adapters for model and persistence may wrap preconstructed services;
plugin lifecycle is mandatory for their publication, without inventing asynchronous
initialization where an adapter needs none. Session file leases remain host-owned
because they protect invocation-level database access, including `sessions`.

## Compatibility and alternatives

The runtime re-exports loop types and port-only composition helpers to minimize
source churn. `Registry::native`, unscoped insertion and standalone
`McpClients::connect` remain conveniences for isolated adapter/contract tests, not
the production composition path. Standalone MCP also publishes discovery batches
transactionally and revokes them on shutdown/drop.

A wholesale domain/ports/kernel crate split was rejected: the new application
boundary enforces the dependency rule without unnecessary fragmentation. Dynamic
Rust libraries, hot reload, WASM isolation and a stable public SDK remain separate
decisions.

## Verification

CI's existing workspace check runs
`python3 .github/scripts/check-huginn-architecture.py`. It inspects Cargo metadata,
including renamed, optional and target-specific declarations, with explicit
allowlists for inward crates. No adapter dependency or local intermediate crate
can be added there without changing the architecture gate. Source checks also
reject direct filesystem/process/network APIs and concrete adapter imports.
`just huginn-architecture` runs the same guard locally; `just huginn-test` runs it
before the focused tests.

Contract tests execute a complete loop with an instruction port and all other
ports implemented in memory, without a real workspace. Lifecycle tests cover
failed activation, invisible staged tools, revocation, stale handles and new
generations. MCP failure injection covers a valid tool followed by an oversized
schema and a successful server followed by a failing server; the registry must
match its pre-activation snapshot. Host policy tests cover exact grants, replacement
prompts, cancellation and unavailable presentation. Existing coding workflow,
persistence, CLI and deterministic/PTY presentation gates remain required.
