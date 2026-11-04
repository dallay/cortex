# Providers: Huginn headless contracts

This delivery implements P0.1 of [ADR-0014](adr/0014-builtin-providers-and-turn-scoped-model-routing.md)
and [#350](https://github.com/dallay/cortex/issues/350). It does not yet implement
`/login`, forms, persistent secrets, or real adapters.

## Composition

`ProvidersPlugin` publishes `agent:provider-catalog@1`, `agent:providers@1`, and the
single `agent:model@1`. Adapters require the catalog and register
via `ProviderCatalog::register(ctx, adapter)` during activation. The host
retains ownership of registrations and tasks; the plugin does not control the terminal.

Registrations are reserved immediately to detect collisions, but are
invisible until `Activation::is_active()` confirms publication. An
activation failure or manifest mismatch releases the reservation. Teardown and
drop revoke retained handles, without substituting them with a new generation.
Identifiers also distinguish catalogs from different supervisors.

Existing builders and `ModelPlugin` keep their signatures: they delegate to
Providers and wrap the configured provider in a legacy ephemeral connection.
An `UnconfiguredModelProvider` is not presented as a ready connection. No
second supervisor is added, nor is the existing CLI/environment configuration changed.

## Selection and authorization

`ProviderAction` offers diagnostics, ephemeral connection, selection, revocation, and
replacement. These are asynchronous headless operations with `CancellationToken`; they do not
authenticate or persist credentials. `ProviderActionState` describes states
that a future host can show while running the actions, without blocking a
UI callback. Actions are serialized before mutating state; prior
cancellation or an obsolete generation cannot modify it.

`ModelProvider::pin_turn` is an additive seam: static providers do not
need to implement it. The router fixes connection, revision, adapter, and model
before waiting on a factory. `StandardLoop` preserves that fixing for
the response, continuation after tools, and automatic compaction. Manual
compaction fixes its own operation. Without a selection, diagnostics
work and inference fails with `ProviderError::Unconfigured`.

Changing the selection affects the next turn. Revoking or replacing the
connection invalidates subsequent inferences, even within the current turn.
Removing the adapter or the core invalidates its generations. There is no
automatic fallback on errors, and a revoked selection remains visible as such
until another connection is explicitly chosen.

Inference admission is serialized with revocation and replacement before
calling the adapter. A request admitted before revocation is an operation
in progress, even if the adapter has not yet been polled.
A revocation after that check locally cancels the operation
on a best-effort basis, both when opening and when consuming streaming. Abandoning the stream
cancels its token. Producers receive cancellation even if the consumer
retains the stream without reading it. Generation watchers belong to the kernel;
no detached tasks are created, and it is not asserted
that the remote server canceled, revoked, or stopped billing.

The token delivered to `ProviderAdapter::create_model` lasts only for the
factory's duration and ends on successful completion too. The returned model must use
the new token from each `stream` call, not retain the factory token as
authorization of the connection.

## Network- and terminal-free demonstration

```sh
cargo run -p huginn-application --example providers_headless
```

Registers two internal demonstration adapters, connects both, fixes one,
selects the other, and verifies that revocation does not trigger fallback. It does not require
provider environment variables, Ratatui, configuration files, or credentials.

## Focused verification

```sh
cargo test -p huginn-core -p huginn-application
cargo test -p huginn-runtime --test stream_persistence --test coding_workflow
cargo test -p huginn --test cli --test config_compat
```

Tests use doubles, signals, and barriers for selection during tools,
compaction, factories, and concurrent turns; they cover rollback, cancellation,
revocation, and obsolete handles. They do not certify secret backends, real
HTTP providers, final onboarding, or human acceptance of the MVP.
