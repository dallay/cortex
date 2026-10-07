# MVP implementation plan

Status: **Implemented; workspace verification and manual acceptance are tracked in validation.md.**  
Audience: personal daily use.  
Platforms: macOS and Linux.  
Product identifier: `agent` (provisional).

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

## Verification

The workspace instruction requires `just ci-local` before claiming implementation
complete. During development, run the narrow Rust checks and tests for the touched
crates, then the full gate. Follow the repository's Semgrep-before-code-generation
rule and run the supply-chain scan after lockfile changes. Verify Rook still builds
and tests without agent dependencies.
