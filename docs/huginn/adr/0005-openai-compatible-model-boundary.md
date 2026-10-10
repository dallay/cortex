# ADR-0005: OpenAI-compatible model boundary

- Status: Accepted
- Date: 2026-10-06
- Product: Cortex agent (provisional name)
- Decision basis: Explicit choices made during the agent MVP planning conversation.

## Context

The agent needs models that can generate structured tool calls and streamed output.
Rook offers an OpenAI-compatible gateway, but compatibility by name is insufficient
evidence for an agent's complete tool-call workflow.

The inspected Rook StreamChunk currently carries text, finish reason and usage,
without a structured tool-call delta representation.

## Decision

The first model adapter targets a configurable OpenAI-compatible endpoint supporting
the agent's required message, tool-call and streaming behavior.

Keep a model-provider contract owned by the agent. Do not import Rook provider-domain
types or couple the agent to a particular endpoint.

Rook is an optional HTTP integration, not a prerequisite for the MVP.
Do not advertise that integration as working until the complete tool-call round trip
has been tested. Any required Rook changes need their own scoped work.

Direct native Anthropic integration and other provider-specific adapters are deferred.

## Alternatives considered

- Require Rook: prevents validating the agent independently and inherits gateway gaps.
- OpenAI and Anthropic adapters immediately: increases initial protocol work.
- Import existing Rook providers: introduces the product-domain coupling rejected by ADR-0001.

## Consequences

- Endpoint compatibility must be demonstrated for tool calls, not only plain text.
- The adapter must retain tool identities and structured arguments, including fragments
  received over streaming.
- A simulated provider supports deterministic tests independently of live credentials.
- Supporting every OpenAI-compatible service is not implied by this choice.

## Follow-up and evidence

Test message/tool-result round trips, fragmented tool-call arguments, multiple tool calls,
malformed responses, stream interruption and cancellation.

Rook's implementation observation comes from
crates/infrastructure/providers-openai/src/provider.rs:151-161, where the
streaming path returns the `content` delta only and drops `tool_calls`. It is
current-state evidence, not an assertion that future Rook versions cannot
support the integration. The full inspection and its consequences for the agent
are recorded in `docs/agent/validation.md` § Rook compatibility result.
