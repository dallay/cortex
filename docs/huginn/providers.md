# Providers: Headless Huginn Contracts

This deliverable implements P0.1 from [ADR-0014](adr/0014-builtin-providers-and-turn-scoped-model-routing.md)
and [#350](https://github.com/dallay/cortex/issues/350). It does not yet implement
`/login`, forms, persistent secrets, or real remote adapters.

## Composition

`ProvidersPlugin` publishes `agent:provider-catalog@1`, `agent:providers@1`, and the
sole `agent:model@1`. Adapters require the catalog and register via
`ProviderCatalog::register(ctx, adapter)` during activation. The host
retains ownership of registrations and tasks; the plugin does not control the terminal.

Registrations are immediately reserved to detect collisions, but remain
invisible until `Activation::is_active()` confirms publication. An activation
failure or discrepancy with the manifest releases the reservation. Teardown and
drop revoke retained handles without replacing them with a new generation.
Identifiers also distinguish catalogs belonging to different supervisors.

Existing builders and `ModelPlugin` maintain their signatures: they delegate to
Providers and wrap the configured provider in a legacy ephemeral connection.
An `UnconfiguredModelProvider` is not presented as a ready connection. No second
supervisor is added, and existing CLI/environment configuration remains unchanged.

## Selection and Authorization

`ProviderAction` provides diagnostics, ephemeral connection, selection, revocation,
and replacement. These are asynchronous headless operations with `CancellationToken`;
they do not authenticate or persist credentials. `ProviderActionState` describes states
that a future host can display when executing actions, without blocking a UI
callback. Actions are serialized before mutating state; prior cancellation
or a stale generation cannot modify it.

`ModelProvider::pin_turn` is an additive seam: static providers do not need
to implement it. The router pins connection, revision, adapter, and model
before awaiting a factory. `StandardLoop` retains that pinned binding for
response generation, continuation after tool calls, and automated compaction. Manual
compaction pins its own operation. Without selection, diagnostics succeed and
inference fails with `ProviderError::Unconfigured`.

Changing selection affects the next turn. Revoking or replacing the
connection invalidates subsequent inferences, even within the current turn.
Removing the adapter or kernel invalidates their generations. There is no automatic
fallback on error, and a revoked selection remains visibly revoked until
another connection is explicitly selected.

Inference admission is serialized with revocation and replacement before
invoking the adapter. A request admitted prior to revocation is an in-flight
operation, even if the adapter has not yet been polled.
Subsequent revocation locally cancels the operation best effort, both when
opening and consuming streams. Dropping the stream cancels its token.
Producers receive cancellation even if the consumer retains the stream without reading.
Generation watchers belong to the kernel; detached tasks are not spawned, and
no claim is made that the remote server has cancelled, revoked, or stopped billing.

The token passed to `ProviderAdapter::create_model` lasts only for the duration
of the factory and completes upon success. The returned model must use the
new token provided to each `stream` call, rather than retaining the factory
token as durable connection authorization.

## Headless Offline Demonstration

```sh
cargo run -p huginn-application --example providers_headless
```

Registers two internal demonstration adapters, connects both, pins one,
selects the other, and verifies that revocation does not trigger a fallback.
Requires no provider environment variables, Ratatui, configuration files, or credentials.

## Focused Verification

```sh
cargo test -p huginn-core -p huginn-application
cargo test -p huginn-runtime --test stream_persistence --test coding_workflow
cargo test -p huginn --test cli --test config_compat
```

Tests use doubles, signals, and barriers to exercise selection during tools,
compaction, factories, and concurrent turns; they cover rollback, cancellation,
revocation, and stale handles. They do not certify secret storage backends,
real HTTP providers, definitive onboarding, or human MVP acceptance.
