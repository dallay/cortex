# ADR-0001: Agent product and documentation boundaries

- Status: Accepted
- Date: 2026-10-06
- Product: Cortex agent (provisional name)
- Decision basis: Explicit choices made during the agent MVP planning conversation.

## Context

Cortex currently contains Rook, an AI gateway, and the research for a future Rust agent.
The Obsidian research still describes repository placement as undecided. We have now
chosen Cortex as the agent's implementation home and the repository as the canonical
home of its implementation decisions.

Rook's existing shared-kernel and provider crates contain Rook domain concepts.
Reusing them as an agent foundation would couple the products.

## Decision

The agent is a separate product inside Cortex, alongside Rook. Its implementation,
configuration, data and eventual releases have their own ownership and identity.

The agent must not depend on Rook domain or application crates, including the current
shared-kernel and providers-core. Rook must not depend on the agent's domain.
Integration between products uses explicit external contracts such as HTTP.
Shared crates are extracted only when a concrete common capability justifies them.

Versioned agent decisions, specifications and backlog live under docs/agent.
Obsidian remains the research home and can link to these documents.
Research observations do not override accepted ADRs.

Use agent as a provisional internal identifier. The public product name remains open.
Independent releases are a product boundary; release automation and public distribution
are not prerequisites for the personal MVP.

## Alternatives considered

- A dedicated repository: stronger isolation, but duplicates tooling before it is needed.
- A temporary prototype in Cortex: leaves product ownership unresolved.
- Building the agent inside Rook: couples distinct domains and release needs.

## Consequences

- Existing Rust tooling and CI can serve both products.
- Rook is optional for agent users.
- Extraction into another repository remains feasible if needed later.
- Existing Rook crates and documentation need not be relocated to begin development.
- Obsidian research must eventually be reconciled with this accepted placement decision.

## Follow-up and evidence

Define the crate layout and eventual release tags when implementing their respective
milestones. Do not treat a provisional identifier as a public branding commitment.

This supersedes the undecided placement recorded as R-008 and OD-010 in the research.
It does not imply that the external research files have already been updated.
