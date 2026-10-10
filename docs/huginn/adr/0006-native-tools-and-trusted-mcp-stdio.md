# ADR-0006: Native tools and trusted MCP over stdio

- Status: Accepted
- Date: 2026-10-06
- Product: Cortex agent (provisional name)
- Decision basis: Explicit choices made during the agent MVP planning conversation.

## Context

Internal composition, external interoperability and sandboxed embedded execution are
different architectural concerns. The research previously left their relationship open
and did not justify WASM as a mandatory external runtime.

MCP over stdio launches local processes. Mediating tool calls does not restrict every
filesystem, network or process action that a server can perform independently.

## Decision

Use native Rust for trusted first-party capabilities and MCP over stdio for external
tool interoperability in the MVP.

MCP servers must be configured explicitly and accepted as trusted software.
Require explicit approval before starting a server, and approval for each tool call
under ADR-0007. Do not execute discovered project-local servers automatically.
Forward only explicitly configured environment variables and required launch configuration;
do not dump the host environment into servers.

Do not promise operating-system sandboxing for MCP processes. They run with the
user's operating-system authority. Host-side approvals govern launches and requested
calls, not all independent server behavior.

WASM, remote MCP transports, OAuth integration, automatic installation and a public
extension registry are deferred.

## Alternatives considered

- Native tools only: postpones real external interoperability.
- Native plus local and remote MCP: adds transport, authentication and operational scope.
- Mandatory sandboxing: adds substantial platform-specific work before the first MCP delivery.
- WASM as a universal extension runtime: has not demonstrated a distinct need for this MVP.

## Consequences

- MCP tools can use existing ecosystem implementations.
- Users must authorize the software they launch, not merely its advertised tool schemas.
- The earlier default-deny research must distinguish governed host APIs from unrestricted
  process authority; its stronger isolation promise cannot be claimed for this adapter.
- The supervisor must own server-process lifecycle and expose failures clearly.

## Follow-up and evidence

Test startup refusal, discovery, approved invocation, cancellation, server failure,
bounded shutdown and environment handling.

The official Rust SDK provides a client-side child-process transport:
[RMCP documentation](https://github.com/modelcontextprotocol/rust-sdk/blob/main/crates/rmcp/README.md).
Dependency version and negotiated protocol version will be pinned during implementation.
