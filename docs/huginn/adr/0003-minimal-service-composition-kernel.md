# ADR-0003: Minimal service composition kernel

- Status: Accepted
- Date: 2026-10-06
- Product: Cortex agent (provisional name)
- Decision basis: Explicit choices made during the agent MVP planning conversation.

**Subsequent decision (2026-10-09):** [ADR-0010](0010-ratatui-plugin-first-interactive-terminal.md) requires that the Ratatui frontend and user-visible features be registered as first-party plugins from the start of the new interactive phase, with an authorable extension path. The original MVP deferral of a stable public SDK, hot reload and native dynamic loading remains historical; it is not a mandate to build monolithic new features.

## Context

The research already accepts composability, dependency-driven lifecycle,
generation-owned effects and a replaceable agent loop. Implementing the whole
plugin framework before a usable agent would delay validation of the product.

## Decision

The MVP includes a real, minimal composition kernel, not only a collection of
statically wired interfaces.

The kernel owns service resolution, plugin lifecycle, generation identity,
effect cleanup and diagnostics. Concrete model access, tools, session storage and
reasoning belong to replaceable capabilities outside the kernel.

Start with one process-level service-resolution scope and one selected provider
per service. Contracts have explicit major versions. Missing hard dependencies
block activation; ambiguous providers and dependency cycles fail explicitly.

Activation must publish registrations only on success and clean up partial effects
on failure. Dependents stop accepting new work and stop before a required provider
is removed. Plugin-owned tasks and registrations are disposed during teardown.

The concrete agent loop is replaceable without changing kernel implementation.
First-party capabilities may be statically linked native Rust.

Hot reload, dynamic native libraries, multiple scopes, provider fan-out and a
public plugin SDK are outside the MVP.

## Alternatives considered

- Complete plugin framework first: delays an end-to-end useful product.
- Simple static wiring followed by a later kernel: defers validation of the main thesis.
- Putting the agent loop in the kernel: makes experimentation require core changes.

## Consequences

- Kernel behavior needs meaningful lifecycle and failure-injection tests.
- Native composition is sufficient to establish plugin semantics.
- Cleanup guarantees apply to registrations and supervised runtime work; they do not
  automatically reverse filesystem writes or other completed external side effects.
- Native code is trusted and does not obtain a sandbox from a service interface.

## Follow-up and evidence

Acceptance must demonstrate blocked dependencies, activation rollback, consumer-before-provider
shutdown, cleanup without orphan registrations or tasks, and replacement of a capability.

The exact Rust trait signatures and crate layout remain implementation details.
This preserves the accepted research principles R-003, R-004, R-006, R-007 and
R-011 through R-018 while narrowing their first delivery.
