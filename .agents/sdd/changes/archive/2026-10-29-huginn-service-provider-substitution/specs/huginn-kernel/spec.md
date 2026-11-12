# Huginn Kernel Specification

> **Domain**: `huginn-kernel`<br>
> **Change**: `huginn-service-provider-substitution`<br>
> **Issue**: [#509](https://github.com/dallay/cortex/issues/509)
> **Type**: New spec

## Purpose

Generic kernel; model and session are tested in the application.

## Requirements

### Requirement: Identity and composition

The kernel MUST identify services by name and explicit major, resolve deterministically, and reject ambiguity or cycles.

#### Scenario: Invalid identifiers and composition

- GIVEN `agent:model@2` requires an `agent:model@1` provider, or there is a duplicated provider/cycle
- WHEN it is resolved or activated
- THEN it fails explicitly and identifies the service or conflict

### Requirement: Transactional activation

The kernel MUST publish after a valid activation, clean up partial effects, assign a generation, and revoke it together with its resources on removal or shutdown.

#### Scenario: Failed activation

- GIVEN a plugin that registers and fails
- WHEN the activation ends
- THEN it does not publish the service and cleans up the effects

### Requirement: Ownership and dependents

The kernel MUST expose read-only queries for provider, generation, and dependents; after the commit it MUST report the new ownership.

#### Scenario: Query after commit

- GIVEN a confirmed substitution
- WHEN the service is queried
- THEN it returns the new provider/generation and the current dependents

### Requirement: Transactional substitution

The kernel MUST substitute without restarting, preserve previous services, and avoid ambiguous, double, or absent ownership. A pre-commit failure MUST preserve the previous one and clean up the alternative. Diagnostics MUST identify the service, the providers, and the outcome; on failure, the reason.

#### Scenario: Successful or failed substitution

- GIVEN a compatible alternative, an incompatible one, or one with an activation failure
- WHEN substitution is requested
- THEN the commit publishes the alternative; if it fails, it preserves the previous one and removes partial records
- AND the diagnostic identifies both providers and the outcome or reason

### Requirement: Cooperative leases with a deadline

The consumer MUST hold the lease during the protected operation. The substitution MUST close admission and wait for leases with a finite deadline; it MUST NOT cancel valid operations. A timeout MUST abort, preserve the previously published and non-revoked service, and reopen admission.

#### Scenario: In-flight operation or timeout

- GIVEN an operation with the previous lease, even after the deadline expires
- WHEN admission is closed or the operation is aborted by timeout
- THEN it is not cancelled; the timeout preserves the previous one and reopens admission
- AND during the close, new leases are rejected

### Requirement: Drain and resumption

`RequireFullDrain` MUST reject dependents without a hook before the close. `ReportUndrained` MAY record them and continue, but MUST wait for quiescence. An abort MUST resume the attempted ones against the previous service; a commit MUST resume them against the new one.

#### Scenario: Drain policy and outcome

- GIVEN a dependent without a hook or one whose drain was attempted
- WHEN it is substituted with `RequireFullDrain`/`ReportUndrained`, and it is aborted or confirmed
- THEN the first policy rejects; the second records and waits; resumption uses the previous one on abort and the new one on commit

### Requirement: Rebinding of application consumers

The productive application/runtime builders (`huginn_runtime::composition::build_native`, `huginn_runtime::composition::build_native_with_connections`, `huginn_runtime::composition::build_native_with_connections_and_guards` and `huginn_application::composition::build`) MUST guarantee that the same `LoopService` retained by the application uses the current providers in the next `run` or `compact` operation that starts after a commit, without rebuilding it. Each active operation MUST keep its previous generations and its leases until it completes; the substitution MUST NOT cancel it nor change its providers halfway through the operation. All session `save` calls of an operation MUST use a single generation and a single store; the next operation after the commit MUST use the new store. The system MUST NOT migrate state between stores.

This guarantee MUST NOT apply to the legacy `LoopPlugin(pub LoopConfig)` nor to custom compositions that only capture services with `resolve() -> Arc<T>`. A custom composition that requires dynamic rebinding MUST opt for `PluginContext::service_slot_handle`, leases, and lifecycle hooks.

#### Scenario: Model rebinding

- GIVEN the application retains a `LoopService` created by one of the productive application/runtime builders listed in this requirement, and an active `run` or `compact` uses the model of the previous generation
- WHEN the substitution waits for the active operation to finish, confirms the new provider, and starts another operation on the same `LoopService`
- THEN the active operation ends without cancellation using the previous model, and the new one uses the new model without rebuilding the `LoopService`

#### Scenario: Session rebinding

- GIVEN the application retains a `LoopService` created by one of the productive application/runtime builders listed in this requirement, and an active `run` or `compact` performs several `save` calls in store A, which contains an identifiable record absent from store B
- WHEN the substitution waits for the active operation to finish, confirms store B, and starts another operation on the same `LoopService`
- THEN all `save` calls of the first operation use the same generation and store A, and the operation ends without cancellation with the previous provider
- AND the next operation uses store B, and the record from A does not appear in B as a result of the substitution

#### Scenario: Legacy or custom composition

- GIVEN `LoopPlugin(pub LoopConfig)` or a custom composition retains an `Arc<T>` obtained with `resolve()`
- WHEN a substitution is confirmed and a later operation starts
- THEN the retained `Arc<T>` remains the captured reference and dynamic rebinding is not guaranteed; a custom composition that requires it uses `PluginContext::service_slot_handle`, leases, and lifecycle hooks

### Requirement: Genericity and compatibility

The kernel MUST NOT know `ModelProvider` nor `SessionStore`. `resolve() -> Arc<T>` is outside the guarantee: arbitrary revocation MUST NOT be asserted. `huginn-plugin-api::v1` MUST remain compatible.

#### Scenario: Existing references and API

- GIVEN a consumer that retains an `Arc<T>` from `resolve()` and consumers of `huginn-plugin-api::v1`
- WHEN the provider is substituted
- THEN revoking the `Arc` is not guaranteed and the existing contracts remain valid
