# huginn-status-bar — Specification

## Purpose

Display real Huginn selection and status without confusing local configuration
with remote availability.

## Requirements

### Requirement: BAR-01 — Persistent Status Bar

The TUI MUST display a compact status bar on every frame, with and without overlays,
including `Approval`. It MUST display current provider, model, and mode or their explicit
absence. An overlay MUST NOT hide it or substitute it with conversational content.
On narrow terminals it MAY abbreviate labels without inventing values or state.

#### Scenario: BAR-01-A — QA with all overlays

- GIVEN a terminal with space for a one-line bar and a known selection
- WHEN rendered without overlay and with each `Overlay` variant, including `Approval`
- THEN the bar remains visible and displays the corresponding provider, model, and mode
- AND closing the overlay does not add those values to the conversation

### Requirement: BAR-02 — Initial Snapshot and Coherent Updates

Core MUST define `Event::ConnectionChanged`; the connections service MUST
publish confirmed provider/model/mode selection and provide an initial snapshot
upon connecting the UI. The bar MUST converge to the active selection without
relying on opening `/providers`. Each update MUST apply as a coherent unit.
Stale asynchronous results, duplicates, or results from a previous connection/generation
MUST NOT overwrite the most recent selection. Failures or cancellations prior to
confirming a change MUST NOT publish a fictitious selection.

#### Scenario: BAR-02-A — Startup and selection absence

- GIVEN a usable persisted selection, or no selection in a separate case
- WHEN the UI connects and receives the initial snapshot
- THEN the bar displays the effective selection or explicit absence without additional interaction
- AND a subsequent confirmed change jointly updates provider, model, and mode

#### Scenario: BAR-02-B — Stale results and cancellation

- GIVEN an active selection S1 and a change in flight to S2
- WHEN S2 fails or is cancelled, or a delayed message from S1 arrives after confirming S2
- THEN the bar preserves the confirmed selection and never displays a transitional cancelled state as active

### Requirement: BAR-03 — Ephemeral Mode and Unavailability Indicators

The bar MUST visibly distinguish ephemeral mode from durable persistence, and
unconfigured state from unavailable credentials. The provider indicator MUST
NOT claim remote availability when only the local configuration exists.
If the terminal lacks space for all indicators, it MUST preserve mode and the
active unconfigured/unavailable indicator ahead of decorative text.

#### Scenario: BAR-03-A — Storage mode and unconfigured state

- GIVEN an ephemeral connection, a durable connection, and an unconfigured state
- WHEN each is active
- THEN the bar explicitly displays ephemeral mode when applicable
- AND unconfigured state displays no fictitious provider name

#### Scenario: BAR-03-B — Narrow terminal degradation

- GIVEN a terminal width of 40 columns
- WHEN the bar renders
- THEN it abbreviates labels while preserving mode and provider/model identity
- AND no visual clipping or layout corruption occurs
