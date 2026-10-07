# ADR-0008: Repository instructions and durable context

- Status: Accepted
- Date: 2026-10-06
- Product: Cortex agent (provisional name)
- Decision basis: Explicit choices made during the agent MVP planning conversation.

## Context

Daily repository work requires project instructions and conversations that survive
application restarts. Long sessions also need a bounded model context without
silently destroying the original history.

## Decision

Include AGENTS.md instructions, persistent sessions and context compaction in the MVP.
Skills discovery and invocation remain outside this delivery.

Apply root repository instructions and relevant nested instructions when working
with files, allowing more specific instructions to refine the general guidance.
Instructions remain task context and cannot authorize effects under ADR-0007.

Persist the original conversation and tool history. Context compaction produces
a separate summary, retains the original record and keeps tool calls paired with
their results. Summarizing a history is not deleting it.

Sessions can be resumed after restarting. Resume must expose interrupted operations
and must not automatically repeat actions with external side effects.

If compaction fails or the context cannot fit safely, stop the affected turn with
an actionable error rather than silently discarding context.

## Alternatives considered

- Instructions without compaction: simpler, but long sessions stop earlier.
- Instructions plus skills immediately: adds discovery, invocation and trust semantics.
- In-memory sessions: fails the restart-and-resume acceptance workflow.
- Replacing history with a summary: loses the audit and recovery record.

## Consequences

- The persisted history and the model's current context are distinct representations.
- Compaction, recovery and cancellation need deterministic tests.
- Session storage must not persist provider credentials.
- Changes to instructions and summary behavior need explicit implementation contracts.

## Follow-up and evidence

Test nested instructions, preserved tool-call/result pairs, compaction failure,
restart/resume, interruption during effects and the absence of automatic effect replay.

SQLite and using the same model provider for summarization were recommended in the
implementation plan. They are implementation proposals, not choices independently
accepted by this ADR. The storage schema, instruction-loading boundaries and context
budget policy must be specified before implementation.
