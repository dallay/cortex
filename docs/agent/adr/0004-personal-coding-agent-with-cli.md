# ADR-0004: Personal coding agent with a CLI

- Status: Accepted
- Date: 2026-10-06
- Product: Cortex agent (provisional name)
- Decision basis: Explicit choices made during the agent MVP planning conversation.

**Partially superseded (2026-10-09):** [ADR-0010](0010-ratatui-plugin-first-interactive-terminal.md) replaces the deferral of the TUI and the long-term selection of a line-oriented CLI for interactive work. This record is retained as the historical decision for the already implemented MVP; its noninteractive, safety, platform and acceptance principles remain relevant. New interactive functionality is Ratatui-first and plugin-first.

## Context

A harness can become a general tool platform, an embeddable SDK, or a coding product.
The first useful release needs a concrete audience and workflow.

## Decision

The first MVP is a local coding agent for personal daily use, tested on macOS and Linux.
Its primary interface is an interactive CLI, with a non-interactive execution mode.

The acceptance workflow is: open a repository, understand a request, inspect files,
propose changes, apply approved changes, execute authorized tests, and resume the
conversation after restarting the application.

Keep the core independent of terminal rendering so future interfaces can consume it.
Installation from source and clear configuration documentation are sufficient for
this first audience.

Defer a TUI, web interface, editor/ACP integration, Windows support, public binary
distribution, skills, subagents, durable workflows and marketplace features.

## Alternatives considered

- A general-purpose tool agent: broader scope and weaker initial acceptance criteria.
- A library/SDK first: prioritizes embedding over an immediately useful coding workflow.
- A TUI or editor integration first: adds interface work before the core workflow is proven.
- A public preview first: requires distribution and onboarding work ahead of personal validation.

## Consequences

- Milestones should produce observable coding behavior rather than only infrastructure.
- The first interface can be simple while streaming, approvals and cancellation remain usable.
- APIs may remain internal and unstable.
- Deferring a feature does not prevent designing a clean future boundary.

## Follow-up and evidence

Validate the complete acceptance workflow on macOS and Linux.
Keep task sequencing and detailed interface contracts in implementation specifications
and backlog rather than in this product-boundary ADR.
