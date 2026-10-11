# ADR-0013: Authorable plugin contract and future loading strategy

- Status: Accepted for the in-process contract; runtime loading deferred
- Date: 2026-10-11
- Product: Huginn
- Relates to: [ADR-0003](0003-minimal-service-composition-kernel.md), [ADR-0010](0010-ratatui-plugin-first-interactive-terminal.md), [ADR-0012](0012-hexagonal-application-and-plugin-lifecycle.md)

## Context

The plugin-first kernel is useful only if built-in capabilities and separately
authored extensions use the same seams. The first interactive implementation
must support authoring without moving terminal, focus, or approval authority
into extensions. Rust dynamic loading, however, introduces an ABI, discovery,
trust, upgrade, and teardown problem that is larger than the current MVP.

## Decision

1. The supported authoring path is a versioned `huginn-plugin-api::v1` contract
   composed into the application as a native Rust dependency.
2. UI contributions are declarative and host-rendered. The host owns TTY input,
   layout, focus, keymaps, and approval policy.
3. `PluginContext` owns generation-scoped registrations and tasks. Publication is
   transactional; activation failure and teardown revoke all owned effects.
4. Native extensions are trusted in-process code. The contract does not claim a
   stable native ABI, dynamic discovery, hot reload, or sandboxing.
5. WASM, IPC/process isolation, and a native loader remain alternatives for a
   future decision. None is selected by this ADR.

## Compatibility and lifecycle analysis

| Option | Compatibility surface | Lifecycle | Isolation | Current decision |
| --- | --- | --- | --- | --- |
| Static native composition | Rust/Cargo types; rebuild required | Supervisor generation cleanup | Trusted process | Selected now |
| Native dynamic loader | ABI, symbol, platform and allocator rules | Hard unload and foreign tasks are difficult | Usually trusted process | Deferred |
| WASM | Host ABI and serialized values | Strong instance boundary if host controls resources | Potential sandbox, subject to runtime policy | Deferred |
| IPC/process | Protocol and deployment compatibility | Process restart and protocol cancellation | OS/process boundary | Deferred |

The next decision must define discovery and installation trust, semver/major
compatibility, capability negotiation, cancellation and task ownership,
state migration, crash handling, resource limits, and whether untrusted code is
allowed. Compatibility with `v1` contribution types should be preserved through
an adapter where practical, but not by freezing a Rust ABI.

## Consequences

Authors can develop an extension in a separate crate and exercise commands,
renderers, dialogs, and widgets without importing TUI internals. Headless modes
remain independent because the UI service is an optional capability. The cost
is static composition and a trusted-code requirement until a future loading
decision is accepted.

## Rejected claims

This ADR does not provide automatic runtime loading, native hot reload,
external sandboxing, or a stable ABI for arbitrary Rust shared libraries.
