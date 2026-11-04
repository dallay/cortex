# Architecture Decision Records (ADRs)

ADRs document significant architecture decisions for the Cortex project.

## General ADRs

> Huginn-specific ADRs are located in [docs/huginn/adr/](../huginn/adr/README.md).

Currently there are no pending general ADRs. Product and architecture ADRs
specific to each component reside in their respective directories:

- [Huginn ADRs](../huginn/adr/) — Decisions regarding the coding agent
- [ADR-0015: Separate product releases from Cargo publishing](../huginn/adr/0015-separate-product-releases-from-cargo-publishing.md) — Distribution policy

## ADR Format

ADRs follow the standard format:

```markdown
# ADR-XXXX: Descriptive Title

- **Status:** [Accepted|Rejected|Proposed|Deprecated]
- **Date:** YYYY-MM-DD
- **Scope:** Context of the ADR

## Summary

Brief description of the problem and the decision.

## Context

The problem or situation motivating this decision.

## Decision

The decision taken and its justification.

## Consequences

- Positive
- Negative
- Neutral
```

See [template.md](template.md) for the complete template.
