# MVP implementation plan

Status: **Implemented; workspace verification and manual acceptance are tracked in validation.md.**  
Audience: personal daily use.  
Platforms: macOS and Linux.  
Product identifier: **Huginn** (official name as of 2026-10-10, see [ADR-0011](adr/0011-huginn-product-naming.md)).

This plan turns the accepted [agent ADRs](adr/README.md) into a build sequence.
Each milestone must leave a working, testable vertical slice and keep Rook buildable.
Rook changes are separate work and require evidence that its HTTP API cannot meet
a specific acceptance case.

## Product acceptance workflow

Given a configured OpenAI-compatible endpoint that supports tool calls and a local
repository, the user can start the CLI, ask for a small code change, let the agent
inspect files, approve a displayed diff, approve a test command, receive streamed
progress and a final response, quit, and resume the session after restart.

Before the MVP is called usable, repeat that workflow on macOS and Linux. A simulated
provider must cover the same orchestration paths deterministically in tests.

## Workspace and crate boundaries

Add an agent application and a small set of product-owned crates to the root Cargo
workspace. Keep Rook's existing crates in place.

Use these responsibilities; combine them only where doing so makes the first slice
simpler without crossing the dependency boundaries:

- **Application:** CLI argument parsing, configuration loading, tracing setup and
  dependency construction. Start with interactive chat, one-shot prompt, session
  listing/resume and diagnostics.
- **Core:** domain types and contracts for model streaming/tool calls, tools and
  approvals, sessions, agent-loop events and replaceable services. No Rook imports
  or terminal I/O.
- **Agent loop:** build model context, stream assistant output, dispatch structured
  tool calls, append tool results and continue until the assistant finishes, errors,
  is cancelled or reaches the configured iteration bound.
- **Native tools:** workspace file listing, reading and search; approved patch
  application; approved shell execution. Resolve paths and symlinks before file
  access and revalidate a proposed patch after approval.
- **Model adapter:** configurable OpenAI-compatible chat-completions endpoint;
  parse tool calls and assemble their fragmented arguments during streaming. Keep
  provider credentials out of logs and persisted session records.
- **Sessions:** durable conversation and event history with resumable interrupted
  state. SQLite is a candidate backend from the planning conversation, not an
  accepted storage decision; settle and record the backend/schema before implementing it.
- **MCP adapter:** configured stdio clients for explicitly trusted servers. The agent
  owns their process lifecycle and asks separately before launch and before each call.
  Document that the host approval layer does not sandbox the child process.

Dependency direction is application → agent capabilities/core; adapters implement
core contracts. No agent crate may depend on `rook-core`, `rook-usecases`,
`providers-core` or Rook's shared kernel. Rook remains an optional HTTP endpoint.

Do not freeze public Rust APIs, plugin manifests, session schemas or the product's
final namespace as part of the MVP. Keep the internal contract small and reviseable.

## Delivery milestones

### 0. Workspace scaffold and core contracts

Add the application and core members to Cargo workspace and establish the provisional
binary name. Define typed errors, cancellation, streaming events and contracts for
the model provider, tool registry, session store, approval policy and replaceable loop.
Add a simulated model and a simple native read-only tool.

Acceptance: workspace builds; the app starts and reports configuration errors clearly;
a deterministic test drives user input → streamed model response → session event.
Dependency checks show no Rook crate in the agent graph.

### 1. Minimal composition lifecycle

Add service registration, required dependency resolution, activation states and an
effect scope for registrations and supervised tasks. First-party implementations can
be statically linked. Publish registrations only after successful activation; tear down
dependants before providers and dispose generation-owned work on shutdown.

Acceptance: tests cover a missing dependency, ambiguous provider, activation rollback,
consumer-before-provider shutdown, task cancellation and no stale registrations.
Do not add hot reload or a general public plugin SDK.

### 2. Coding loop and terminal experience

Implement the conversational loop with streamed text, structured tool-call assembly,
sequential tool execution, results returned to the model, user cancellation and a
bounded maximum number of iterations. Add interactive and one-shot CLI modes.

Register native list/read/search tools first. Add patch display and explicit approval
before applying writes; require approval before shell execution. In non-interactive
mode, deny unapproved effects.

Acceptance: deterministic tests cover text-only turns, one or multiple tool calls,
malformed arguments, model errors, denial, cancellation, stale diff and workspace
path/symlink escape attempts.

### 3. Real OpenAI-compatible adapter

Support configurable base URL, model, API credential source, request limits and
timeouts. Verify the endpoint supports structured tool use and streaming. Keep Rook
optional; test its actual endpoint compatibility before describing it as supported.

Acceptance: adapter tests use a local mock HTTP server and cover fragmented tool
arguments, malformed responses, provider errors, timeouts and cancellation. A manual
smoke test with a compatible service verifies the full coding loop.

### 4. Repository instructions and durable sessions

Load root and applicable nested `AGENTS.md` instructions. Instructions are prompt
context only and never grant a capability or approval.

Persist messages, tool calls/results, approvals and interruption state. Resume must
show unfinished work and must never replay a possibly effectful action automatically.
Add context compaction that stores a separate summary and preserves the original history
and complete tool-call/result pairs. Stop safely if summarization fails or context
cannot fit. Never persist API credentials.

Acceptance: tests cover nested instructions, restart/resume, tool-pair preservation,
compaction, compaction failure and interruption during an approved effect.

### 5. Trusted MCP stdio and daily-use validation

Use the official Rust MCP SDK client transport. Load servers from explicit user
configuration; require launch approval and approval per tool call. Pass only configured
environment values, supervise cancellation and shutdown, and surface server failures.

Acceptance: tests cover denied startup/call, successful discovery and invocation,
server crash, timeout and bounded shutdown. Complete the product acceptance workflow
on macOS and Linux using both a local MCP fixture and the model endpoint.

## Implementation choices

Storage, configuration, protocol version, path containment, invocation grants and
context budgets are now specified in [ADR-0009](adr/0009-sqlite-config-and-mvp-runtime-contracts.md)
and the [implementation specification](implementation-specification.md).
The implementation exists; full acceptance requires the [validation record](validation.md),
a real model smoke test and Linux daily-use validation.

## Next interactive implementation phase — Ratatui-first plugin architecture

**Current governing decision:** [ADR-0010](adr/0010-ratatui-plugin-first-interactive-terminal.md). Milestones 0–5 above describe the implemented historical line-oriented MVP, **not** the design pattern for new interactive features. Ratatui development begins in this phase, in parallel with remaining daily-use validation rather than after completing public packaging.

Delivery order:

1. **Presentation composition boundary:** add a Ratatui presentation plugin in an isolated crate/module and have the composition root choose exactly one interactive owner. Keep the line-oriented CLI available with an explicit override; never initialize both input readers. `huginn-core`, `huginn-runtime` and Rook remain free of Ratatui dependencies.
2. **Inline vertical slice:** Ratatui/Crossterm/Tokio renderer, multiline composer, session resume, streaming text/tool output, approval dialogs invoking the existing `ApprovalPolicy`, cancellation and terminal restoration. Maintain `run`, `--json`, `doctor` and `sessions` behavior.
3. **Extensible UX:** expose lifecycle-owned host-rendered contributions for commands, tool renderers, status and dialogs. Demonstrate two swappable built-in contributions registered/unregistered via the kernel. Treat future user-authored plugins and a versioned extension developer contract as first-class roadmap requirements; do not prematurely freeze a dynamic ABI.
4. **Correctness/performance:** pin versions and exercise real-PTY rapid resize, scrollback, cursor query/input races, Unicode, modal focus, permission fail-closed, external process handoff when implemented, stalled renderers and terminal recovery. Use an isolated terminal adapter or upstream fix if stock Ratatui exposes an actual defect; write no speculative fork.
5. **Release acceptance:** only make interactive TUI the default after the smoke/PTY matrix passes on the supported terminal targets; until then, the existing CLI remains functional. Record measured evidence in `validation.md`.

The current synchronous `EventSink` must be adapted through a reliable, nonblocking projection to TUI state with coalesced redraw notifications, rather than one redraw per token or an uncontrolled second terminal reader. A pending framework-specific issue is not an excuse to extend the line-oriented CLI as the new UI architecture.

## Verification

The workspace instruction requires `just ci-local` before claiming implementation
complete. During development, run the narrow Rust checks and tests for the touched
crates, then the full gate. Follow the repository's Semgrep-before-code-generation
rule and run the supply-chain scan after lockfile changes. Verify Rook still builds
and tests without agent dependencies.
