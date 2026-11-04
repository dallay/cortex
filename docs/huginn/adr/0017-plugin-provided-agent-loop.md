# ADR-0017: Plugin-provided agent loop

- Status: Accepted
- Date: 2026-11-03
- Product: Huginn
- Relates to: [ADR-0003](0003-minimal-service-composition-kernel.md), [ADR-0007](0007-read-freely-approve-effects.md), [ADR-0012](0012-hexagonal-application-and-plugin-lifecycle.md), [ADR-0013](0013-authorable-plugin-contract-and-loading.md), [ADR-0016](0016-restrict-only-tool-hooks.md)
- Decision basis: Explicit maintainer decision to make the loop a kernel plugin in the DALLAY-720 refinement; implemented with substitution and conformance tests.

## Context

ADR-0003 already states that the concrete agent loop is replaceable without changing
kernel implementation, and the host publishes it as `agent:loop@1` through
`LoopService`. Two facts made that replaceability nominal rather than real:

- The service id lived in `huginn_application::composition`. A plugin outside that
  crate could implement `AgentLoop` but could not name the service it must provide
  without taking a dependency on the application crate.
- The author-facing contract, `huginn_plugin_api::v1`, exposed `hooks` and `tools`
  only. There was no authorable path to the loop at all, so "replaceable through a
  plugin" was satisfied by nothing but a test double.

The host also needs a guarantee that survives replacement: authorization, effects,
events, session persistence and cancellation are host-owned. A loop that could
approve its own effects, or own its own session store, would break ADR-0007 and the
no-replay rule.

## Decision

1. `huginn_core::agent_loop_id()` owns the `agent:loop@1` service id.
   `huginn_application::composition::loop_id()` delegates to it, so existing callers
   and tests keep their behavior unchanged.
2. `huginn_plugin_api::v1::agent` re-exports the author-facing loop contract:
   `AgentLoop`, `LoopService`, `agent_loop_id`, and the host types a `run` receives.
3. A plugin that implements `AgentLoop` is installed either by static composition
   instead of the host loop plugin, or at runtime through
   `Supervisor::substitute(&agent_loop_id(), alternative, policy, timeout)`. Duplicate
   providers still fail explicitly; substitution drains the previous loop first, so
   two loops never execute a turn at the same time.
4. The host retains authority. A loop receives the approval policy, event sink,
   cancellation token, tool registry, instruction source and session store; it never
   owns them. `AgentLoop::run` can request a decision but cannot authorize an effect,
   because only `ApprovalPolicy` and the host tools grant effects (ADR-0007).
5. Turn identity is allocated by the host-owned `Session::allocate_turn()`, so every
   implementation reports compatible turn ids that survive resume. Step identity is
   the loop's responsibility: every step-scoped event it emits must carry the
   `StepId` of the model call that produced it.
6. A replacement loop is a trusted native in-process component, exactly like a native
   tool hook. This decision adds no sandbox and no dynamic loading.

## Alternatives considered

- **Dynamic loading of loop implementations.** Deferred by ADR-0013; the trust and
  ABI questions are unchanged, and no requirement here needs runtime code loading.
- **Let a loop own approvals.** Rejected: that is a second authority path beside
  `HostPolicy` and would bypass denial guarantees.
- **One service id per loop flavour.** Rejected: ambiguous providers must fail
  explicitly, so the kernel keeps exactly one active provider per service.
- **Let a loop own its session store.** Rejected: replay safety depends on the host
  persisting each turn through one store, and the leased loop already demonstrates
  that service handles are the right boundary.

## Consequences

- Any loop implementation is now certifiable by one behavioral and security suite
  instead of by reading its source (tracked as #534).
- The host loop keeps its drain and lease guarantees; an alternative loop must declare
  `supports_drain` if it holds resources that need a quiescent handover.
- A loop implementation that forgets step identity produces uncorrelatable events
  rather than a compile error, so the conformance suite is the enforcement point.
- `huginn-plugin-api` gains one versioned module; no existing author contract changes.

## Follow-up and evidence

- Tests: `crates/huginn/plugin-api/tests/agent_loop.rs` (authorable path) and
  `crates/huginn/application/tests/plugin_loop.rs` (substitution, no double
  invocation).
- Still open: a second distinct loop implementation, turn-level inference hooks,
  steering, budget enforcement and context pinning are tracked separately under
  DALLAY-720.
