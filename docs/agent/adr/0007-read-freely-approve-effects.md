# ADR-0007: Read freely and approve effects

- Status: Accepted
- Date: 2026-10-06
- Product: Cortex agent (provisional name)
- Decision basis: Explicit choices made during the agent MVP planning conversation.

## Context

The coding agent needs useful autonomy while preserving the user's control over
changes and commands. Native code and trusted MCP servers can perform effects,
so approval behavior must be explicit at the host boundary.

## Decision

Within an authorized repository workspace, allow native read and search operations
without repeated approval.

Require approval for filesystem modifications, shell commands and every MCP tool call.
Present a diff before an edit and the concrete command or tool arguments before execution.
MCP startup approval is separate from approval of an invocation.

Instructions, model output and tool metadata cannot grant permissions.
Resolve paths and symlinks so native filesystem tools cannot silently escape
the authorized workspace.

After approval, revalidate the content that an edit will change. If it changed,
produce a new diff and request new approval rather than applying an obsolete proposal.

In non-interactive mode, deny actions needing approval unless the user explicitly
authorized those actions for that execution. Do not silently enable unrestricted access.

## Alternatives considered

- Automatic edits with shell approval: less interruption but less control over changes.
- Broad session autonomy: faster execution with a larger authorization scope.
- Approval for every read: adds friction without matching the chosen coding workflow.

## Consequences

- Approval is an explicit execution state shared by the CLI and the core.
- Denial produces a clear result that the loop can handle without executing the action.
- Native path enforcement is not a confinement guarantee for an approved shell command
  or MCP process; ADR-0006 explains that process trust boundary.
- An instruction file cannot bypass the policy.

## Follow-up and evidence

Test denied edits, commands and MCP calls; expired or changed edit proposals;
workspace escapes through symlinks; and non-interactive execution without authorization.

Detailed approval identifiers, persistence and scoped non-interactive authorization
syntax remain to be specified before implementing these interfaces.
