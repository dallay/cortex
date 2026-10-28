# ADR-0016: Restrict-only tool hooks

- Status: Proposed
- Date: 2026-10-27
- Product: Huginn
- Relates to: [ADR-0007](0007-read-freely-approve-effects.md), [ADR-0012](0012-hexagonal-application-and-plugin-lifecycle.md), [ADR-0013](0013-authorable-plugin-contract-and-loading.md)
- Decision basis: Implemented with lifecycle, fail-closed and loop-integration tests; awaiting product acceptance.

## Context

Plugins can replace the whole loop or contribute UI, but they cannot react to an
individual tool call. Guards such as destructive-command detection, sensitive
path protection or context-budget notices would otherwise require replacing the
loop or wrapping every tool. ADR-0007 makes `HostPolicy` the only authority over
effects, so any interception point must not become a second approval path.

## Decision

1. `huginn-core::hooks` defines `ToolHook` with two optional methods:
   - `before(call, ctx)` returns `ToolVerdict::Continue` or `ToolVerdict::Block(reason)`.
   - `after(call, outcome, ctx)` may return a note for the model-visible result.
2. No verdict authorizes an action. Hooks can only restrict, observe or annotate.
3. The loop evaluates `before` hooks after tool lookup and **before**
   `prepare`, approval and execution. A blocked call produces no preview, no
   approval prompt and no effect. It is returned to the model as a tool error.
4. Hooks run sequentially in hook-id order and the first block wins.
5. A hook error, panic or timeout (default 2 s per hook) blocks the call
   (fail closed). After an effect has run, a failure becomes a visible note
   instead, because it cannot undo the effect.
6. Hook reasons and notes are stripped of control and bidi characters and are
   bounded to 4 KiB each.
7. `ToolHooks` is a live registry published as `agent:tool-hooks@1` by the
   host-owned `core.hooks` plugin. Registrations are owned by the plugin
   generation: they become visible on commit and are removed on failure,
   removal or shutdown.
8. Authors use `huginn_plugin_api::v1::hooks`. The trust model of ADR-0013 is
   unchanged: native hooks are trusted in-process code and this is not a sandbox.

## Alternatives considered

- **Allow hooks to approve.** Rejected: it would bypass ADR-0007 and turn every
  plugin into a policy authority.
- **Run hooks after `prepare`.** Rejected for now: preparation can read files
  and build previews, and a guard should be able to stop a call before any of that.
- **Concurrent hook evaluation.** Rejected: ordering would become
  nondeterministic and a block could race with other hooks' side effects.

## Consequences

- Guards (destructive commands, sensitive paths) and annotations (context
  budget, commit-size signals) can ship as independent plugins.
- Every hook adds latency to every tool call. The per-hook timeout bounds a
  stuck hook, not the cost of a slow one.
- A panic in a native hook is contained by the loop, but the process panic hook
  still runs. Hooks must not panic, as with other native callbacks.

## Follow-up and evidence

- Tests: `crates/huginn/core/tests/tool_hooks.rs`,
  `crates/huginn/application/tests/tool_hooks.rs` and
  `crates/huginn/plugin-api/tests/hooks.rs`.
- Still open: turn-level hooks, context contributors, plugin-originated
  follow-up messages and a dedicated audit event for blocks.
