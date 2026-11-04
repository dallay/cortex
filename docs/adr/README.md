# Architecture Decision Records (ADRs)

ADRs document significant architecture decisions for the Cortex project.

## General ADRs

> Huginn-specific ADRs live in [docs/huginn/adr/](huginn/adr/README.md).

There are currently no pending general ADRs. Product- and architecture-specific
ADRs for each component live in their respective directories:

- [Huginn ADRs](huginn/adr/) — Decisions about the coding agent
- [ADR-0015: Separation of product releases and Cargo publishing](../huginn/adr/0015-separate-product-releases-from-cargo-publishing.md) — Distribution policy

## ADR format

ADRs follow the standard format:

```markdown
# ADR-XXXX: Descriptive title

- **Status:** [Accepted|Rejected|Proposed|Deprecated]
- **Date:** YYYY-MM-DD
- **Scope:** Context of the ADR

## Summary

Brief description of the problem and the decision.

## Context

The problem or situation motivating this decision.

## Decision

The decision made and its justification.

## Consequences

- Positive
- Negative
- Neutral
```

See [template.md](template.md) for the complete template.
