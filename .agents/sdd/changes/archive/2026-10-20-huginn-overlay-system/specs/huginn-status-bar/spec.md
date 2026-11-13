# huginn-status-bar — Specification

## Purpose

Show the real selection and state of Huginn without confusing local
configuration with remote availability.

## Requirements

### Requirement: BAR-01 — Persistent bar

The TUI MUST show a compact bar on every frame, with and without an overlay,
including `Approval`. It MUST show the current provider, model, and mode, or
their explicit absence. An overlay MUST NOT hide it nor replace it with
conversational content. In narrow terminals it MAY abbreviate labels without
inventing values or state.

#### Scenario: BAR-01-A — QA with all overlays

- GIVEN a terminal with room for one bar line and a known selection
- WHEN it is rendered without an overlay and with each variant of `Overlay`, including `Approval`
- THEN the bar stays visible and shows the corresponding provider, model, and mode
- AND closing the overlay does not add those values to the conversation

### Requirement: BAR-02 — Initial snapshot and coherent updates

The core MUST define `Event::ConnectionChanged`; the connections service MUST
publish the confirmed provider/model/mode selection and provide an initial
snapshot when the UI connects. The bar MUST converge to the current selection
without depending on opening `/providers`. Each update MUST be applied as a
coherent unit. Stale, duplicated, or previous-connection/generation asynchronous
results MUST NOT overwrite the most recent selection. Failures or cancellations
before a change is confirmed MUST NOT publish a fictitious selection.

#### Scenario: BAR-02-A — Startup and absence of selection

- GIVEN a usable persisted selection, or no selection in a separate case
- WHEN the UI connects and receives the initial snapshot
- THEN the bar shows the effective selection or the explicit absence without further interaction
- AND a later confirmed change updates provider, model, and mode together

#### Scenario: BAR-02-B — Out-of-order update

- GIVEN confirmed changes A and then B, with the bar already showing B
- WHEN an update of A arrives late, or an initial snapshot older than B arrives
- THEN the bar keeps B without mixing A's provider with B's model/mode
- AND a duplication of B produces no notices nor additional conversation

#### Scenario: BAR-02-C — Failure, revocation, and reconnection

- GIVEN a selection A and an attempt to change to B
- WHEN B fails or is canceled, or A is revoked, in separate cases
- THEN the failure/cancellation does not announce B as a confirmed selection
- AND the revocation reflects the remaining effective selection or its absence
- AND after the UI reconnects, events from the previous generation do not revert the current snapshot

### Requirement: BAR-03 — Connection evidence, not local inference

Selecting a provider/model or retrieving credentials MUST NOT be enough to show
`Connected` or an equivalent of verified remote availability. The UI MUST
distinguish local selection/preparation from remote evidence. A remote success
MAY be described as such only for the verified connection and operation;
changing the selection, revoking credentials, or receiving a failure MUST NOT
preserve a misleading claim of a current connection. The bar MUST reflect
activity, pending approval, and errors without turning them into conversation
messages.

#### Scenario: BAR-03-A — QA without remote evidence

- GIVEN a locally selected provider/model without successful remote calls
- WHEN the TUI opens or `/model` completes
- THEN the local selection is shown without `Connected` nor any other promise of remote availability
- AND a candidate model is not described as discovered or remotely validated

#### Scenario: BAR-03-B — Stale evidence and error

- GIVEN successful remote evidence of A
- WHEN B is selected without verification, A is revoked, or a current remote operation fails, in separate cases
- THEN the bar does not attribute A's previous availability to the current selection
- AND the error or pending state remains observable even with an overlay open
