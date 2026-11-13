# Huginn Microkernel Specification

> **Domain**: `huginn-microkernel`<br>
> **Change**: `huginn-epic-01-microkernel-service-composition`<br>
> **Epic**: [#442](https://github.com/dallay/cortex/issues/442) / Linear `DALLAY-718`<br>
> **Slice**: [#509](https://github.com/dallay/cortex/issues/509)<br>
> **Type**: New spec

## Purpose

Product-level contract of the Huginn EPIC 01 microkernel: a minimal kernel that composes
replaceable services without embedding product-specific behavior.

This spec states the epic capabilities and the epic acceptance outcome. It does **not** restate
kernel mechanics. The normative runtime behavior lives in `.agents/sdd/specs/huginn-kernel/spec.md`,
and every requirement below cites the kernel requirement that carries the detail.

Architectural boundary (from the epic): model providers, session stores, tools, Agent Loop and UIs
remain **replaceable capabilities**, not privileged kernel implementations.

## Requirements

### Requirement: Versioned contracts and deterministic service resolution

The composition MUST identify every service by name and explicit major version, MUST resolve a
service to exactly one provider, and MUST fail explicitly on ambiguous providers, dependency cycles
and unsatisfied hard dependencies. Resolution MUST be deterministic for a given composition.

Normative detail: `huginn-kernel` → "Identity and composition".

#### Scenario: Ambiguous or cyclic composition

- GIVEN two plugins provide the same `ServiceId`, or the manifests form a dependency cycle
- WHEN the composition is resolved or activated
- THEN it fails explicitly and identifies the conflicting service or cycle

#### Scenario: Deterministic activation order

- GIVEN a set of plugins with a valid dependency graph
- WHEN the composition is activated
- THEN the activation order is deterministic and identical for identical inputs

### Requirement: Supervisor lifecycle and reversible registrations

The supervisor MUST publish a plugin's registrations only after a successful activation, MUST
clean partial effects when an activation fails, MUST assign a generation to each activation, and
MUST revoke that generation together with the resources it owns on removal, replacement or
shutdown.

Normative detail: `huginn-kernel` → "Transactional activation".

#### Scenario: Failed activation leaves no published service

- GIVEN a plugin that registers a service and then fails
- WHEN the activation ends
- THEN the service is not published and the partial effects are cleaned up

#### Scenario: Revocation releases owned resources

- GIVEN an activated plugin that owns a task or a resource
- WHEN the plugin is removed, replaced or the supervisor is dropped
- THEN the owned resource is released with its generation

### Requirement: Ownership, dependency introspection and scoped authority

The kernel MUST expose read-only introspection of which plugin provides a service, which plugins
depend on it, and the generation in effect. It MUST offer a per-operation authority handle whose
admission the kernel can close, and it MUST report the provider and the dependents after a
successful substitution.

Normative detail: `huginn-kernel` → "Ownership and dependents", "Cooperative leases with a
deadline".

#### Scenario: Introspection before and after a substitution

- GIVEN a published service with known dependents
- WHEN the service is queried before and after a confirmed substitution
- THEN it returns the current provider, generation and dependents in both states

#### Scenario: Authority is scoped to an operation

- GIVEN a consumer that holds a lease for the duration of one operation
- WHEN the kernel closes admission for a substitution
- THEN new leases are rejected while the held lease remains valid until the operation completes

### Requirement: Explicit replacement of service providers with safe rollback

The kernel MUST offer one substitution primitive that replaces the active provider plugin of a
published service. The alternative MUST be activated transactionally and MUST publish only on
success. If activation or the bounded drain fails, the substitution MUST abort, MUST clean the
partial effects, MUST reopen admission on the previous provider, and MUST report an actionable
diagnostic naming the service, the previous provider, the alternative and the failure reason. The
kernel MUST NOT cancel valid in-flight operations to force a replacement.

Normative detail: `huginn-kernel` → "Transactional substitution", "Drain and resumption",
"Cooperative leases with a deadline".

#### Scenario: Successful substitution

- GIVEN an active provider and a compatible alternative
- WHEN the substitution is requested
- THEN the alternative is published only after the previous provider is quiescent, and no state
  exists in which the service resolves to two providers or to none

#### Scenario: Failed substitution preserves the previous provider

- GIVEN an alternative that fails to activate, or a dependent that does not quiesce before the
  deadline
- WHEN the substitution is requested
- THEN the previous provider remains resolvable and effective, admission is reopened, no partial
  registration survives, and the diagnostic reports the reason

#### Scenario: Dependents resume against the effective provider

- GIVEN dependents of a replaced service
- WHEN the substitution aborts or commits
- THEN the dependents resume against the previous provider on abort and against the new provider
  on commit

### Requirement: Capability-agnostic composition boundary

The kernel MUST NOT contain knowledge of models, tools, sessions, providers or terminal concerns,
and `huginn-core` MUST NOT gain terminal, HTTP, SQLite or Rook dependencies. The author-facing
plugin contract MUST remain backward compatible when the kernel surface changes.

Normative detail: `huginn-kernel` → "Genericity and compatibility".

#### Scenario: Kernel stays capability-agnostic

- GIVEN a substituted service is a model provider in one composition and a session store in
  another
- WHEN both compositions run the same kernel mechanism
- THEN neither composition requires a kernel change, and `huginn-plugin-api::v1` consumers remain
  valid

### Requirement: Epic acceptance outcome

An alternative service implementation MUST be substitutable at runtime while consumers, authority
checks and lifecycle guarantees continue to work: consumers observe exactly one valid provider at
all times, authority checks keep holding operations to their leases, and the lifecycle guarantees
of transactional activation, rollback and per-generation revocation MUST remain in force.

Normative detail: `huginn-kernel` → "Transactional substitution", "Cooperative leases with a
deadline", "Drain and resumption", "Rebinding of application consumers".

#### Scenario: Substitution preserves consumers, authority and lifecycle

- GIVEN a running composition with a retained consumer, an operation in flight and a lifecycle
  contract
- WHEN an alternative implementation is substituted
- THEN the in-flight operation completes on the previous provider without being cancelled, the next
  operation uses the new provider, the retained consumer is rebound where the composition opted
  into rebinding, and a failed substitution leaves the previous provider and its dependents intact

## Deferred scope

The following items are known gaps of EPIC 01. They are explicitly deferred in slice issue #509 and
are **not** part of the epic acceptance outcome:

- A kernel-level lifecycle event stream for subscribers. Typed events exist at loop scope
  (`huginn_core::Event`, `EventSink`, `ScopedEventSink`), but the kernel does not publish
  discovered, activated, deactivated, replaced, blocked or failed composition events.
- Semantic-version compatibility rules beyond the explicit major version of a `ServiceId`.
- Multi-candidate provider selection. The kernel resolves exactly one provider per service and
  rejects ambiguity.
- A resolution scope in the sense of a namespaced registry. Authority is scoped per operation
  through cooperative leases and slot handles, not through a scoped namespace.
