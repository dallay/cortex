# Agent architecture decision records

These records capture decisions accepted during the agent MVP planning on
2026-10-06. Accepted means the direction was chosen; it does not mean the agent
is implemented or its behavior has been verified.

The product name is still open. Agent is a provisional internal identifier.

## Decision index

| ADR | Decision | Status |
| --- | --- | --- |
| [ADR-0001](0001-agent-product-and-documentation-boundaries.md) | Agent product and documentation boundaries | Accepted |
| [ADR-0002](0002-own-rust-core-with-selective-reuse.md) | Own Rust core with selective reuse | Accepted |
| [ADR-0003](0003-minimal-service-composition-kernel.md) | Minimal service composition kernel | Accepted |
| [ADR-0004](0004-personal-coding-agent-with-cli.md) | Personal coding agent with a CLI | Accepted |
| [ADR-0005](0005-openai-compatible-model-boundary.md) | OpenAI-compatible model boundary | Accepted |
| [ADR-0006](0006-native-tools-and-trusted-mcp-stdio.md) | Native tools and trusted MCP over stdio | Accepted |
| [ADR-0007](0007-read-freely-approve-effects.md) | Read freely and approve effects | Accepted |
| [ADR-0008](0008-repository-instructions-and-durable-context.md) | Repository instructions and durable context | Accepted |
| [ADR-0009](0009-sqlite-config-and-mvp-runtime-contracts.md) | SQLite, configuration and runtime contracts | Accepted |

## Record format

Use the [template](template.md) for new records. Each ADR covers a coherent decision,
its context, alternatives, consequences and follow-up evidence.
Keep implementation tasks and API specifications in separate documents linked to
the relevant ADRs.

Use Proposed for a choice still awaiting a decision, Accepted for an agreed choice,
Rejected for an evaluated choice not selected, and Superseded for a replaced decision.
Accepted decisions are replaced by a new ADR referencing the old record; preserve
the old rationale and add a supersession link rather than rewriting its history.

Number new ADRs sequentially. Do not change their identity when the product gets a name.

## Documentation authority and research

This directory is the canonical versioned decision log for the agent in Cortex.
The Obsidian rust-agent-harness-research collection remains the research home;
its DEC and R decision entries provide historical context.

The repository placement and MCP trust decisions here refine earlier research
that left placement undecided or assumed stronger external-runtime isolation.
The external research notes now link to these canonical MVP decisions; their older
alternatives are preserved as research history.

## Remaining decisions

- Public product name and final crate/configuration namespaces.
- Supported real endpoint combinations and Rook compatibility.
- Release tags and public distribution after personal MVP acceptance.

Concrete contracts and limits are in the [implementation specification](../implementation-specification.md).
ADR-0009 records the storage and configuration choices made for implementation.

## MVP validation boundary

The personal MVP targets macOS and Linux and the complete coding workflow described
in ADR-0004. Kernel lifecycle, tool-call streaming, approvals, MCP failures, context
compaction and interruption/recovery need focused acceptance checks.

Implementation follows the repository quality gates, including just ci-local.
Writing an ADR does not satisfy those gates or constitute an implemented feature.
