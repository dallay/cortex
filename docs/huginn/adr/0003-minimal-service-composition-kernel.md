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

## Subsequent decision — cooperative provider substitution (2026-10-29)

The supervisor can substitute an active service provider without restarting the
process. The alternative must continue to publish all services from the previous
provider. It can add services only if no other registered plugin declares
those IDs. This preserves the existing service set and allows safe
extension.

Before the commit, the supervisor closes lease admission for the previous
generation, runs the available drain hooks, and waits for active leases to
finish within a single finite deadline. The hook deadline and the lease wait
do not cancel valid operations protected by leases.
`RequireFullDrain` rejects the request if any active dependent of any
service from that generation lacks a drain hook. `ReportUndrained` allows
those dependents and records them. If draining fails, the lease deadline expires, or
the alternative activation fails, the previous generation reopens and the
dependents whose draining was attempted resume. The kernel records the result of the
substitution and the IDs of drained and undrained dependents.

If the substitution future is cancelled before commit, the supervisor reopens the
previous gates and schedules cleanup of the alternative and resumption of attempted
dependents in the current runtime. After commit, finalization runs inline, not in a
background task. Dropping the future at that point leaves the alternative active,
can interrupt retired-plugin cleanup or dependent resumption, and prevents recording
the substitution diagnostic.

`Supervisor::substitute` can return `Err` after commit when retired-provider
deactivation or dependent resumption fails. The alternative remains active. Callers
must inspect `provider_of(service)` and the corresponding entry in
`substitution_diagnostics()` instead of treating this error as a rollback. When the
`Supervisor` is dropped, the kernel revokes its gates and clears its slots, so
retained handles cannot acquire new leases. The lifecycle order places the
alternative after its required providers and before its existing dependents. The
supervisor rejects a substitution when no position satisfies both constraints.

Leases are a cooperative API contract, not strong revocation. The
consumer must hold the lease for the whole operation and must not retain
an internal reference to the provider after releasing the lease. The legacy
`resolve() -> Arc<T>` method remains available, but it does not offer this guarantee.
The kernel does not guarantee revocation of retained `Arc`s or detached work.

This decision does not add capability-specific behavior to
`huginn-core`, does not change `huginn-plugin-api::v1`, and does not promise to revoke
arbitrary provider references. It refines the earlier requirement that dependents
stop accepting work before a required provider is retired.

## Subsequent decision — production slots and loop rebind (2026-10-29)

The kernel keeps a global slot table per `ServiceId`. A
`ServiceSlotHandle<T>` retains the table and the ID; each `acquire()` resolves the
current value and gate and increments its lease under the same synchronization. The
commit replaces all slots of the generation in a critical section. A
pre-commit abort restores the previous provider, reopens the gates, and resumes the
dependents whose drain was attempted. The timeout does not cancel active operations.

In production, `huginn_application::composition::build` and the native builders of
`huginn_runtime::composition` publish the private `LeasedAgentLoop` adapter
via the `leased_loop_plugin` factory and `LeasedLoopPlugin`. `run` and `compact`
obtain model and session admission and leases before producing effects, and keep
them until the operation completes. All `save` calls within an operation use
the same store. Draining closes admission and waits for active operations;
`resume` reopens it after abort or commit. If the second lease fails, RAII releases
the first one without mutating the session or producing effects. The concrete plugin
type remains private.

Public compatibility takes precedence over converting the legacy loop:
`StandardLoop` keeps its six public fields, signatures, and struct literals;
`LoopPlugin(pub LoopConfig)` keeps its constructor. Direct use of those
types publishes the static loop and does not obtain a rebind guarantee. The
production guarantee belongs to the builders that register `LeasedLoopPlugin` via
`leased_loop_plugin`.

Readiness checks for doctor, CLI, and TUI acquire the current `ModelService` slot on
each check and fail closed if it cannot be acquired. They do not retain an
`Arc<ModelProvider>` or a captured readiness boolean. This dynamic read
does not extend the cooperative guarantee of leases.

`SqliteSessions::lease`, `load`, and `list` remain outside `SessionsService` and
go directly to the `SqliteSessions` adapter. Substitution does not transfer locks or
migrate state to the new store. No database migration or schema
change is added.

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
